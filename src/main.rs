use std::{
	collections::HashSet,
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
	process::Command as Process,
};

use cargo_metadata::MetadataCommand;
use clap::{Arg, ArgAction, Command};
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

fn main() {
	let matches = Command::new("rust-cleanup")
		.arg(
			Arg::new("path")
				.short('p')
				.long("path")
				.value_name("PATH")
				.help("Sets the starting directory for the search"),
		)
		.arg(
			Arg::new("yes-cargo")
				.long("yes-cargo")
				.action(ArgAction::SetTrue)
				.help("Automatically clean non-Dioxus Rust projects without prompting"),
		)
		.arg(
			Arg::new("yes-dioxus")
				.long("yes-dioxus")
				.action(ArgAction::SetTrue)
				.help("Automatically clean Dioxus projects without prompting"),
		)
		.arg(
			Arg::new("yes-all")
				.long("yes-all")
				.short('y')
				.action(ArgAction::SetTrue)
				.help("Automatically clean all projects without prompting for a yes or a no"),
		)
		.arg(
			Arg::new("orphans")
				.long("orphans")
				.action(ArgAction::SetTrue)
				.help(
					"Also remove Cargo build dirs that aren't inside any discovered project \
					 (e.g. left over from `cargo build --target-dir <dir>`)",
				),
		)
		.get_matches();

	let start_path = matches
		.get_one::<String>("path")
		.map_or(".", String::as_str);
	let flags = Flags {
		yes_cargo: matches.get_flag("yes-cargo"),
		yes_dioxus: matches.get_flag("yes-dioxus"),
		yes_all: matches.get_flag("yes-all"),
		orphans: matches.get_flag("orphans"),
	};

	let discovery = discover(Path::new(start_path));
	let workspaces = build_plan(&discovery.candidates);

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

	print_summary(already_clean, detached_found, flags.orphans, &skipped);
}

#[derive(Clone, Copy)]
struct Flags {
	yes_cargo: bool,
	yes_dioxus: bool,
	yes_all: bool,
	orphans: bool,
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

	let mut it = WalkDir::new(start).into_iter();
	while let Some(next) = it.next() {
		let Ok(entry) = next else { continue };
		if !entry.file_type().is_dir() {
			continue;
		}
		let path = entry.path();

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

	Discovery {
		candidates,
		build_dirs,
	}
}

/// Resolves the candidates into a set of distinct clean jobs by asking `cargo
/// metadata` for each project's authoritative workspace root and build
/// directory. Members of an already-resolved workspace are folded into it rather
/// than queried again.
fn build_plan(candidates: &[Candidate]) -> Vec<Workspace> {
	// Shallow directories first: a workspace root is always shallower than its
	// members, so we resolve the root (and learn its members) before we would ever
	// reach a member on its own.
	let mut order: Vec<&Candidate> = candidates.iter().collect();
	order.sort_by_key(|c| c.dir.components().count());

	let mut workspaces: Vec<Workspace> = Vec::new();
	let mut covered: HashSet<PathBuf> = HashSet::new();

	for candidate in order {
		if covered.contains(&candidate.dir) {
			continue;
		}

		match resolve_workspace(&candidate.dir) {
			Some((root, target_dir, member_dirs)) => {
				workspaces.push(Workspace {
					root,
					target_dir,
					kind: workspace_kind(candidate.kind, &member_dirs),
				});
				covered.extend(member_dirs);
			}
			None => {
				// `cargo metadata` failed (bad manifest, cargo missing, ...). Fall back
				// to the safe default so the project is never silently ignored.
				eprintln!(
					"Warning: `cargo metadata` failed for {}; assuming a standalone project with a `target/` build dir.",
					candidate.dir.display(),
				);
				workspaces.push(Workspace {
					root: candidate.dir.clone(),
					target_dir: candidate.dir.join("target"),
					kind: candidate.kind,
				});
			}
		}

		// Account for the triggering directory itself (a virtual workspace root is
		// not a package, so it won't appear among the members).
		covered.insert(candidate.dir.clone());
	}

	workspaces
}

/// Runs `cargo metadata` for a manifest and returns `(workspace_root,
/// target_directory, member_dirs)`. Returns `None` if cargo could not read the
/// project.
fn resolve_workspace(dir: &Path) -> Option<(PathBuf, PathBuf, Vec<PathBuf>)> {
	let metadata = MetadataCommand::new()
		.manifest_path(dir.join("Cargo.toml"))
		// Run from inside the project so cargo discovers project-local and global
		// config (e.g. `[build] target-dir`); config is resolved relative to the
		// working directory, not the manifest path.
		.current_dir(dir)
		.no_deps()
		.exec()
		.ok()?;

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

	Some((root, target_dir, member_dirs))
}

/// A workspace counts as Dioxus if the triggering crate or any member carries a
/// `Dioxus.toml`, so `--yes-dioxus` applies to it.
fn workspace_kind(trigger_kind: ProjectType, member_dirs: &[PathBuf]) -> ProjectType {
	let has_dioxus = trigger_kind == ProjectType::Dioxus
		|| member_dirs.iter().any(|d| d.join("Dioxus.toml").exists());
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

fn print_summary(already_clean: usize, detached_found: usize, orphans_enabled: bool, skipped: &[PathBuf]) {
	if already_clean > 0 {
		println!("{already_clean} project(s) were already clean.");
	}
	if detached_found > 0 && !orphans_enabled {
		println!(
			"Found {detached_found} orphaned Cargo build dir(s) not tied to any project; re-run with --orphans to remove them.",
		);
	}
	if !skipped.is_empty() {
		println!("Skipped:");
		for path in skipped {
			println!("  {}", path.display());
		}
	}
}
