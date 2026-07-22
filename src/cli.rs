use std::{
	collections::HashSet,
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
	process::Command as Process,
	time::Duration,
};

use clap::{Args, Parser};
use cargo_metadata::MetadataCommand;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use walkdir::{DirEntry, WalkDir};

/// The cache-directory tag cargo drops into every build directory. Its body
/// contains the word "cargo" (`... created by cargo.`), which lets us tell a
/// Cargo build dir apart both from an unrelated directory merely named `target`
/// and from other tools' generic `CACHEDIR.TAG` caches. We only ever delete a
/// directory that carries this cargo-authored tag.
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";

/// Directory names we never descend into: large, noisy trees that never hold a
/// Cargo project. (Build directories are pruned dynamically via `CACHEDIR_TAG`,
/// so `target` is intentionally not listed here — it might have been renamed.)
const PRUNED_DIRS: [&str; 3] = [".git", "node_modules", ".jj"];


/// Frees disk space by cleaning the build artifacts of Rust projects under a
/// directory. Already-clean projects are skipped, workspaces are cleaned once,
/// and stray/orphaned build dirs are detected by cargo's `CACHEDIR.TAG`.
#[derive(Parser)]
#[command(name = "rust-cleanup", version, about)]
pub struct Cli {
	/// Sets the starting directory for the search
	#[arg(short, long, value_name = "PATH", default_value = ".")]
	pub path: PathBuf,

	#[command(flatten)]
	pub flags: Flags,
}

/// The auto-clean / behavior flags, shared with the cleaning logic. Kept as a
/// small `Copy` bundle so it can be threaded through by value.
#[derive(Args, Clone, Copy)]
pub struct Flags {
	/// Automatically clean non-Dioxus Rust projects without prompting
	#[arg(long)]
	pub yes_cargo: bool,

	/// Automatically clean Dioxus projects without prompting
	#[arg(long)]
	pub yes_dioxus: bool,

	/// Automatically clean all projects without prompting for a yes or a no
	#[arg(short = 'y', long)]
	pub yes_all: bool,

	/// Also remove Cargo build dirs that aren't inside any discovered project
	/// (e.g. left over from `cargo build --target-dir <dir>`)
	#[arg(long)]
	pub orphans: bool,

	/// Show what would be cleaned without deleting anything or prompting
	#[arg(short = 'n', long)]
	pub dry_run: bool,

	/// List the projects that `cargo metadata` could not read
	#[arg(short, long)]
	pub verbose: bool,
}



pub fn run_cli() {
	let cli = Cli::parse();
	let flags = cli.flags;

	// `cargo metadata` calls are subprocess/I/O-bound, so we oversubscribe the
	// core count to keep more of them in flight. Skip this if the user pinned
	// RAYON_NUM_THREADS, and ignore the error if a pool already exists.
	if std::env::var_os("RAYON_NUM_THREADS").is_none() {
		let threads = std::thread::available_parallelism()
			.map_or(8, |n| n.get())
			.saturating_mul(2)
			.clamp(4, 32);
		let _ = rayon::ThreadPoolBuilder::new()
			.num_threads(threads)
			.build_global();
	}

	let discovery = discover(&cli.path);
	let (workspaces, failed) = build_plan(&discovery.candidates);

	if flags.dry_run {
		println!("Dry run — nothing will be deleted.");
	}

	let mut skipped: Vec<PathBuf> = Vec::new();
	let mut already_clean = 0usize;

	// Pass 1: clean each distinct workspace/standalone project once, using the
	// build directory cargo itself resolved (so relocated build dirs are handled).
	// Dedupe by resolved build dir in case several projects share one.
	let mut cleaned: HashSet<PathBuf> = HashSet::new();
	for ws in &workspaces {
		if !ws.target_dir.is_dir() {
			already_clean += 1;
			continue;
		}
		if !cleaned.insert(canonical_or(&ws.target_dir)) {
			continue; // this build dir was already cleaned via another project
		}

		if flags.dry_run {
			println!(
				"Would clean {} ({} project) — build dir {}",
				ws.root.display(),
				ws.kind.display_name(),
				ws.target_dir.display(),
			);
			continue;
		}

		let question = format!(
			"{} is a {} project (build dir: {}). Clean it?",
			ws.root.display(),
			ws.kind.display_name(),
			ws.target_dir.display(),
		);
		if ws.kind.should_autoclean(flags) || prompt(&question) {
			cargo_clean(&ws.root);
		} else {
			skipped.push(ws.root.clone());
		}
	}

	// Pass 2: every Cargo build dir we found that ISN'T some project's resolved
	// build dir is a leftover — a renamed `target/`, a dir from an old global
	// `build.target-dir`, or a one-off `--target-dir`. `cargo clean` never touches
	// these, so we remove them directly.
	let resolved: HashSet<PathBuf> = workspaces
		.iter()
		.map(|ws| canonical_or(&ws.target_dir))
		.collect();
	let mut detached_found = 0usize;

	for build_dir in &discovery.build_dirs {
		if resolved.contains(build_dir) {
			continue; // the real build dir of a project — handled by pass 1
		}

		match containing_project(build_dir, &discovery.candidates) {
			// Leftover sitting inside a known project: confidently that project's.
			Some(project) => {
				if flags.dry_run {
					println!(
						"Would remove stray build dir {} (inside {})",
						build_dir.display(),
						project.dir.display(),
					);
					continue;
				}
				let question = format!(
					"{} is a stray Cargo build dir (not {}'s current build dir). Remove it?",
					build_dir.display(),
					project.dir.display(),
				);
				if project.kind.should_autoclean(flags) || prompt(&question) {
					remove_dir(build_dir);
				} else {
					skipped.push(build_dir.clone());
				}
			}
			// Not inside any project: ambiguous, so only touch it behind --orphans.
			None => {
				if !flags.orphans {
					detached_found += 1;
					continue;
				}
				if flags.dry_run {
					println!("Would remove orphaned build dir {}", build_dir.display());
					continue;
				}
				let question = format!(
					"{} is an orphaned Cargo build dir with no associated project. Remove it?",
					build_dir.display(),
				);
				if flags.yes_all || prompt(&question) {
					remove_dir(build_dir);
				} else {
					skipped.push(build_dir.clone());
				}
			}
		}
	}

	print_summary(&Summary {
		already_clean,
		detached_found,
		orphans_enabled: flags.orphans,
		skipped: &skipped,
		failed: &failed,
		verbose: flags.verbose,
	});
}



#[derive(PartialEq, Clone, Copy)]
enum ProjectType {
	Rust,
	Dioxus,
}

impl ProjectType {
	/// Classifies a Cargo project directory. Every Dioxus project is also a Cargo
	/// project, so `Dioxus.toml` just refines a directory that already has a
	/// `Cargo.toml`.
	fn detect(dir: &Path) -> Option<Self> {
		if !dir.join("Cargo.toml").exists() {
			return None;
		}
		if dir.join("Dioxus.toml").exists() {
			Some(Self::Dioxus)
		} else {
			Some(Self::Rust)
		}
	}

	fn display_name(self) -> &'static str {
		match self {
			Self::Rust => "Rust",
			Self::Dioxus => "Dioxus",
		}
	}

	fn should_autoclean(self, flags: Flags) -> bool {
		flags.yes_all
			|| match self {
				Self::Rust => flags.yes_cargo,
				Self::Dioxus => flags.yes_dioxus,
			}
	}
}

struct Candidate {
	dir: PathBuf,
	kind: ProjectType,
}

/// A distinct thing to clean: one standalone crate or one workspace, together
/// with the build directory cargo resolved for it.
struct Workspace {
	root: PathBuf,
	target_dir: PathBuf,
	kind: ProjectType,
}

struct Discovery {
	candidates: Vec<Candidate>,
	/// Every cargo-authored build directory found, canonicalized.
	build_dirs: Vec<PathBuf>,
}

/// Walks the tree once. Prunes VCS/dependency trees by name and every cache
/// directory by its `CACHEDIR.TAG`, recording the cargo-authored ones as build
/// directories and collecting every Cargo project.
fn discover(start: &Path) -> Discovery {
	let mut candidates = Vec::new();
	let mut build_dirs = Vec::new();

	// walkdir can't report a total up front (it discovers dirs as it goes), so the
	// walk gets a spinner with live counts rather than a percentage bar.
	let spinner = ProgressBar::new_spinner();
	spinner.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
	spinner.enable_steady_tick(Duration::from_millis(100));
	let mut scanned = 0u64;

	let mut it = WalkDir::new(start).into_iter();
	while let Some(next) = it.next() {
		let Ok(entry) = next else { continue };
		if !entry.file_type().is_dir() {
			continue;
		}
		let path = entry.path();

		scanned += 1;
		if scanned.is_multiple_of(128) {
			spinner.set_message(format!(
				"Scanning… {scanned} dirs, {} project(s), {} build dir(s)",
				candidates.len(),
				build_dirs.len(),
			));
		}

		if is_pruned(&entry) {
			it.skip_current_dir();
			continue;
		}

		// A directory carrying CACHEDIR.TAG is a cache and never contains projects,
		// so prune it. If cargo wrote the tag, it's a build dir worth cleaning.
		if let Ok(tag) = fs::read_to_string(path.join(CACHEDIR_TAG)) {
			if tag.contains("cargo") {
				if let Ok(dir) = path.canonicalize() {
					build_dirs.push(dir);
				}
			}
			it.skip_current_dir();
			continue;
		}

		if let Some(kind) = ProjectType::detect(path) {
			if let Ok(dir) = path.canonicalize() {
				candidates.push(Candidate { dir, kind });
			}
		}
	}

	spinner.finish_and_clear();
	println!(
		"Scanned {scanned} directories: found {} project(s) and {} build dir(s).",
		candidates.len(),
		build_dirs.len(),
	);

	Discovery { candidates, build_dirs }
}

/// Resolves the candidates into a set of distinct clean jobs by asking `cargo
/// metadata` for each project's authoritative workspace root and build
/// directory. Members of an already-resolved workspace are folded into it rather
/// than queried again.
///
/// The `cargo metadata` calls dominate the runtime, so they run in parallel. The
/// "skip detached children" optimization needs workspace roots resolved before
/// their non-member children can be judged, so we do it in two parallel batches:
/// first every workspace root (identified by a cheap `[workspace]` line-scan,
/// no cargo spawn), then the remaining standalone crates once coverage is known.
fn build_plan(candidates: &[Candidate]) -> (Vec<Workspace>, Vec<(PathBuf, String)>) {
	// The slow phase: one `cargo metadata` per uncovered project. We know the total
	// up front, so this gets a real progress bar with an ETA.
	let progress = ProgressBar::new(candidates.len() as u64);
	progress.set_style(
		ProgressStyle::with_template("{spinner:.green} Resolving projects [{bar:30.cyan/blue}] {pos}/{len} ({eta})")
			.unwrap()
			.progress_chars("=>-"),
	);
	progress.enable_steady_tick(Duration::from_millis(120));

	// Batch 1: resolve every workspace root in parallel.
	let (roots, non_roots): (Vec<&Candidate>, Vec<&Candidate>) = candidates
		.iter()
		.partition(|c| manifest_is_workspace_root(&c.dir));
	let root_results = resolve_in_parallel(&roots, &progress);

	let mut workspaces: Vec<Workspace> = Vec::new();
	let mut failed: Vec<(PathBuf, String)> = Vec::new();
	let mut covered: HashSet<PathBuf> = HashSet::new();
	// Directories of workspaces we've handled (resolved or broken). Their
	// non-member descendants are crates cargo rejects as detached; we skip them.
	let mut handled_roots: Vec<PathBuf> = Vec::new();

	for (candidate, result) in root_results {
		match result {
			Ok((root, target_dir, member_dirs)) => {
				handled_roots.push(canonical_or(&root));
				covered.extend(member_dirs.iter().cloned());
				workspaces.push(Workspace {
					root,
					target_dir,
					kind: workspace_kind(candidate.kind, &member_dirs),
				});
			}
			// A broken workspace root; record it and skip its members below.
			Err(e) => {
				handled_roots.push(candidate.dir.clone());
				failed.push((candidate.dir.clone(), e));
			}
		}
		covered.insert(candidate.dir.clone());
	}

	// Batch 2: the remaining crates that aren't members of, or detached children
	// of, a workspace we already handled. A crate inside a handled workspace but
	// not among its members is one cargo refuses to resolve ("believes it's in a
	// workspace when it's not"); its build dirs are still caught by the scan, so we
	// don't waste a `cargo metadata` call on it.
	let mut skipped = 0u64;
	let standalone: Vec<&Candidate> = non_roots
		.into_iter()
		.filter(|c| {
			let keep = !covered.contains(&c.dir)
				&& !handled_roots
					.iter()
					.any(|root| c.dir.starts_with(root) && c.dir != *root);
			if !keep {
				skipped += 1;
			}
			keep
		})
		.collect();
	progress.inc(skipped);

	for (candidate, result) in resolve_in_parallel(&standalone, &progress) {
		match result {
			// We don't fabricate a build dir on failure: the CACHEDIR.TAG scan handles
			// any real one, whereas a fake resolved target would mask it from the scan.
			Ok((root, target_dir, member_dirs)) => workspaces.push(Workspace {
				root,
				target_dir,
				kind: workspace_kind(candidate.kind, &member_dirs),
			}),
			Err(e) => failed.push((candidate.dir.clone(), e)),
		}
	}

	progress.finish_and_clear();

	// Dedupe workspaces that share a resolved root (e.g. members of a workspace
	// whose root lives above the search path, each resolved on its own).
	let mut seen_roots = HashSet::new();
	workspaces.retain(|ws| seen_roots.insert(canonical_or(&ws.root)));

	(workspaces, failed)
}

/// Runs `cargo metadata` for each candidate in parallel, advancing `progress`
/// as results land. Errors are stringified for later reporting.
#[allow(clippy::type_complexity)]
fn resolve_in_parallel<'a>(
	candidates: &[&'a Candidate],
	progress: &ProgressBar,
) -> Vec<(&'a Candidate, Result<(PathBuf, PathBuf, Vec<PathBuf>), String>)> {
	candidates
		.par_iter()
		.map(|candidate| {
			let result = resolve_workspace(&candidate.dir).map_err(|e| e.to_string());
			progress.inc(1);
			(*candidate, result)
		})
		.collect()
}

/// Runs `cargo metadata` for a manifest and returns `(workspace_root,
/// target_directory, member_dirs)`, or the error if cargo could not read it.
fn resolve_workspace(dir: &Path) -> Result<(PathBuf, PathBuf, Vec<PathBuf>), cargo_metadata::Error> {
	let metadata = MetadataCommand::new()
		.manifest_path(dir.join("Cargo.toml"))
		// Run from inside the project so cargo discovers project-local and global
		// config (e.g. `[build] target-dir`); config is resolved relative to the
		// working directory, not the manifest path.
		.current_dir(dir)
		.no_deps()
		.exec()?;

	let root = metadata.workspace_root.into_std_path_buf();
	let target_dir = metadata.target_directory.into_std_path_buf();
	// With `--no-deps`, `packages` is exactly the set of workspace members.
	let member_dirs = metadata
		.packages
		.iter()
		.filter_map(|pkg| pkg.manifest_path.parent())
		.map(|dir| {
			let dir = dir.to_path_buf().into_std_path_buf();
			dir.canonicalize().unwrap_or(dir)
		})
		.collect();

	Ok((root, target_dir, member_dirs))
}

/// Whether `dir`'s `Cargo.toml` declares a workspace (`[workspace]` or any
/// `[workspace.*]` table). Members inherit via `field.workspace = true`, which
/// is not a table header, so this doesn't match them.
fn manifest_is_workspace_root(dir: &Path) -> bool {
	let Ok(contents) = fs::read_to_string(dir.join("Cargo.toml")) else {
		return false;
	};
	contents
		.lines()
		.map(str::trim)
		.any(|line| line == "[workspace]" || line.starts_with("[workspace."))
}

/// A workspace counts as Dioxus if the triggering crate or any member carries a
/// `Dioxus.toml`, so `--yes-dioxus` applies to it.
fn workspace_kind(trigger_kind: ProjectType, member_dirs: &[PathBuf]) -> ProjectType {
	let has_dioxus = trigger_kind == ProjectType::Dioxus || member_dirs.iter().any(|d| d.join("Dioxus.toml").exists());
	if has_dioxus {
		ProjectType::Dioxus
	} else {
		ProjectType::Rust
	}
}

/// The deepest project directory that contains `build_dir`, if any.
fn containing_project<'a>(build_dir: &Path, candidates: &'a [Candidate]) -> Option<&'a Candidate> {
	candidates
		.iter()
		.filter(|c| build_dir.starts_with(&c.dir) && build_dir != c.dir.as_path())
		.max_by_key(|c| c.dir.components().count())
}

/// Whether the walk should refuse to descend into `entry`. The start directory
/// (depth 0) is always kept.
fn is_pruned(entry: &DirEntry) -> bool {
	entry.depth() > 0
		&& entry
			.file_name()
			.to_str()
			.is_some_and(|name| PRUNED_DIRS.contains(&name))
}

fn canonical_or(path: &Path) -> PathBuf {
	path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn remove_dir(dir: &Path) {
	if let Err(e) = fs::remove_dir_all(dir) {
		eprintln!("Failed to remove {}: {e}", dir.display());
	}
}

fn cargo_clean(dir: &Path) {
	match Process::new("cargo").arg("clean").current_dir(dir).status() {
		Ok(status) if !status.success() => {
			eprintln!("`cargo clean` exited with a nonzero status in {}", dir.display());
		}
		Err(e) => eprintln!("Failed to run `cargo clean` in {}: {e}", dir.display()),
		Ok(_) => {}
	}
}

/// Asks a yes/no question, re-prompting on unrecognized input. A closed stdin
/// (EOF) or read error is treated as "no" so we can't spin forever.
fn prompt(question: &str) -> bool {
	print!("{question} (y/n): ");
	io::stdout().flush().ok();

	loop {
		let mut input = String::new();
		match io::stdin().read_line(&mut input) {
			Ok(0) | Err(_) => return false,
			Ok(_) => {}
		}

		match input.trim().to_lowercase().as_str() {
			"y" | "yes" => return true,
			"n" | "no" => return false,
			_ => {
				print!("Please answer y or n: ");
				io::stdout().flush().ok();
			}
		}
	}
}

struct Summary<'a> {
	already_clean: usize,
	detached_found: usize,
	orphans_enabled: bool,
	skipped: &'a [PathBuf],
	failed: &'a [(PathBuf, String)],
	verbose: bool,
}

fn print_summary(summary: &Summary) {
	if summary.already_clean > 0 {
		println!("{} project(s) were already clean.", summary.already_clean);
	}
	if !summary.failed.is_empty() {
		println!(
			"{} project(s) couldn't be read by `cargo metadata` (detached/broken manifests); their build dirs are still handled by the direct scan.",
			summary.failed.len(),
		);
		if summary.verbose {
			for (dir, err) in summary.failed {
				let reason = err.lines().next().unwrap_or("").trim();
				println!("  {}: {reason}", dir.display());
			}
		} else {
			println!("  (re-run with -v to list them)");
		}
	}
	if summary.detached_found > 0 && !summary.orphans_enabled {
		println!(
			"Found {} orphaned Cargo build dir(s) not tied to any project; re-run with --orphans to remove them.",
			summary.detached_found,
		);
	}
	if !summary.skipped.is_empty() {
		println!("Skipped:");
		for path in summary.skipped {
			println!("  {}", path.display());
		}
	}
}
