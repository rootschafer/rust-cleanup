use std::{
	collections::HashSet,
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
	process::Command as Process,
	time::{Duration, SystemTime},
};

use crate::{
	cli::Flags,
	discover::{Discovery, containing_project},
	plan::Workspace,
	util::canonical_or,
};

/// Executes the plan: clean each project's resolved build dir, then remove any
/// stray/orphaned build dir the scan turned up, then print a summary. Honors
/// `--dry-run` by reporting instead of deleting.
pub(crate) fn run(flags: Flags, discovery: &Discovery, workspaces: &[Workspace], failed: &[(PathBuf, String)]) {
	if flags.dry_run {
		println!("Dry run — nothing will be deleted.");
	}

	let mut tally = Tally::default();
	let mut already_clean = 0usize;

	// Pass 1: clean each distinct workspace/standalone project once, using the
	// build directory cargo itself resolved (so relocated build dirs are handled).
	// Dedupe by resolved build dir in case several projects share one.
	let mut cleaned: HashSet<PathBuf> = HashSet::new();
	for ws in workspaces {
		if !ws.target_dir.is_dir() {
			already_clean += 1;
			continue;
		}
		if !cleaned.insert(canonical_or(&ws.target_dir)) {
			continue; // this build dir was already cleaned via another project
		}
		tally.consider(Target::Project { root: &ws.root }, &ws.target_dir, flags);
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
		match containing_project(build_dir, &discovery.projects) {
			// Leftover sitting inside a known project: confidently that project's.
			Some(project) => tally.consider(Target::Stray { project }, build_dir, flags),
			// Not inside any project: ambiguous, so only touch it behind --orphans.
			None if flags.orphans => tally.consider(Target::Orphan, build_dir, flags),
			None => detached_found += 1,
		}
	}

	if flags.show_size {
		let verb = if flags.dry_run { "Would free" } else { "Freed" };
		println!("{verb} ~{}.", human_size(tally.freed));
	}

	print_summary(&Summary {
		already_clean,
		kept_by_filter: tally.kept_by_filter,
		detached_found,
		orphans_enabled: flags.orphans,
		skipped: &tally.skipped,
		failed,
		verbose: flags.verbose,
	});
}

/// What a build directory is to us, which decides both how we describe it and
/// how it gets removed.
enum Target<'a> {
	/// A project's current build dir — `cargo clean` handles it, from `root`.
	Project { root: &'a Path },
	/// A leftover build dir sitting inside a project it no longer belongs to.
	Stray { project: &'a Path },
	/// A build dir with no project around it at all.
	Orphan,
}

/// What the run added up to, accumulated as build dirs are handled.
#[derive(Default)]
struct Tally {
	/// Estimated bytes cleaned (only meaningful with `--show-size`).
	freed: u64,
	kept_by_filter: usize,
	skipped: Vec<PathBuf>,
}

impl Tally {
	/// Runs one build dir through the whole decision: measure if a filter or
	/// `--show-size` needs it, apply the filters, then report it (`--dry-run`),
	/// delete it (`--yes`), or ask. Every removal in the program goes through here.
	fn consider(&mut self, target: Target, build_dir: &Path, flags: Flags) {
		let measured = measure_if_needed(build_dir, flags);
		if !filters_allow(measured, flags) {
			self.kept_by_filter += 1;
			return;
		}
		let size = measured.map_or(0, |m| m.bytes);
		let sized = with_size(build_dir, measured, flags);

		if flags.dry_run {
			match &target {
				Target::Project { root } => {
					println!("Would clean {} — build dir {sized}", root.display())
				}
				Target::Stray { project } => {
					println!("Would remove stray build dir {sized} (inside {})", project.display())
				}
				Target::Orphan => println!("Would remove orphaned build dir {sized}"),
			}
			self.freed += size;
			return;
		}

		let question = match &target {
			Target::Project { root } => {
				format!("{} is a Cargo project (build dir: {sized}). Clean it?", root.display())
			}
			Target::Stray { project } => format!(
				"{sized} is a stray Cargo build dir (not {}'s current build dir). Remove it?",
				project.display(),
			),
			Target::Orphan => format!("{sized} is an orphaned Cargo build dir with no associated project. Remove it?"),
		};

		if !(flags.yes || prompt(&question)) {
			// Name the project for a project, the directory itself otherwise — that's
			// what the user just declined.
			self.skipped.push(match target {
				Target::Project { root } => root.to_path_buf(),
				_ => build_dir.to_path_buf(),
			});
			return;
		}

		let removed = match target {
			Target::Project { root } => cargo_clean(root),
			Target::Stray { .. } | Target::Orphan => remove_dir(build_dir),
		};
		// Only count what actually went away, so "Freed ~X" can't be inflated by a
		// removal that failed.
		if removed {
			self.freed += size;
		}
	}
}

/// What `measure` learned about a build dir. When `complete` is false, part of
/// the tree couldn't be read, so `bytes` and `newest` are lower bounds.
#[derive(Clone, Copy)]
struct Measurement {
	bytes: u64,
	newest: SystemTime,
	complete: bool,
}

/// Measures a build dir only when a filter or `--show-size` needs it; otherwise
/// returns `None` so a normal run pays nothing.
fn measure_if_needed(build_dir: &Path, flags: Flags) -> Option<Measurement> {
	let needed = flags.keep_days.is_some() || flags.keep_size.is_some() || flags.show_size;
	needed.then(|| measure(build_dir))
}

/// Renders a build-dir path, appending its human size in parentheses when
/// `--show-size` is on and the size is known. An incomplete measurement is a
/// lower bound, shown as such.
fn with_size(path: &Path, measured: Option<Measurement>, flags: Flags) -> String {
	match (flags.show_size, measured) {
		(true, Some(m)) => {
			let bound = if m.complete { "" } else { "≥ " };
			format!("{} ({bound}{})", path.display(), human_size(m.bytes))
		}
		_ => path.display().to_string(),
	}
}

/// Formats a byte count as a 1024-based human string (e.g. `1.5 GiB`).
fn human_size(bytes: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
	let mut value = bytes as f64;
	let mut unit = 0;
	while value >= 1024.0 && unit < UNITS.len() - 1 {
		value /= 1024.0;
		unit += 1;
	}
	if unit == 0 {
		format!("{bytes} B")
	} else {
		format!("{value:.1} {}", UNITS[unit])
	}
}

/// Whether a build dir survives the `--keep-days` / `--keep-size` filters and may
/// therefore be cleaned, given its (optional) measurement. Returns `true`
/// immediately when no filter is set. Note the polarity: we *keep* (return
/// `false` for) recently-touched or small build dirs, and only clean stale/large
/// ones. A missing or incomplete measurement keeps the dir: an unreadable part of
/// the tree could be arbitrarily recent or large, so we never delete something we
/// failed to fully inspect.
fn filters_allow(measured: Option<Measurement>, flags: Flags) -> bool {
	if flags.keep_days.is_none() && flags.keep_size.is_none() {
		return true;
	}
	let Some(m) = measured else {
		return false;
	};
	if !m.complete {
		return false;
	}
	if let Some(days) = flags.keep_days
		&& touched_within(m.newest, days)
	{
		return false; // recently built — protect active work
	}
	if let Some(min_size) = flags.keep_size
		&& m.bytes < min_size
	{
		return false; // too small to be worth reclaiming
	}
	true
}

/// Total size in bytes and newest file mtime under `dir`. Walks the whole tree
/// (the inherent cost of size/age filtering); does not follow symlinks. Any
/// entry it fails to read marks the measurement incomplete rather than silently
/// counting as zero — `filters_allow` depends on that honesty.
fn measure(dir: &Path) -> Measurement {
	let mut m = Measurement {
		bytes: 0,
		newest: SystemTime::UNIX_EPOCH,
		complete: true,
	};
	let mut stack = vec![dir.to_path_buf()];

	while let Some(current) = stack.pop() {
		let Ok(entries) = fs::read_dir(&current) else {
			m.complete = false;
			continue;
		};
		for entry in entries {
			let Ok(entry) = entry else {
				m.complete = false;
				continue;
			};
			let Ok(file_type) = entry.file_type() else {
				m.complete = false;
				continue;
			};
			if file_type.is_dir() {
				stack.push(entry.path());
			} else if file_type.is_file() {
				let Ok(meta) = entry.metadata() else {
					m.complete = false;
					continue;
				};
				m.bytes += meta.len();
				if let Ok(modified) = meta.modified()
					&& modified > m.newest
				{
					m.newest = modified;
				}
			}
		}
	}

	m
}

/// Whether `mtime` is within the last `days` days (clock skew into the future is
/// treated as "recent").
fn touched_within(mtime: SystemTime, days: u64) -> bool {
	match SystemTime::now().duration_since(mtime) {
		Ok(elapsed) => elapsed < Duration::from_secs(days.saturating_mul(86_400)),
		Err(_) => true,
	}
}

/// Removes `dir`, reporting whether it actually went away.
fn remove_dir(dir: &Path) -> bool {
	if let Err(e) = fs::remove_dir_all(dir) {
		eprintln!("Failed to remove {}: {e}", dir.display());
		return false;
	}
	true
}

/// Runs `cargo clean` in `dir`, reporting whether it succeeded.
fn cargo_clean(dir: &Path) -> bool {
	match Process::new("cargo").arg("clean").current_dir(dir).status() {
		Ok(status) if !status.success() => {
			eprintln!("`cargo clean` exited with a nonzero status in {}", dir.display());
			false
		}
		Err(e) => {
			eprintln!("Failed to run `cargo clean` in {}: {e}", dir.display());
			false
		}
		Ok(_) => true,
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
	kept_by_filter: usize,
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
	if summary.kept_by_filter > 0 {
		println!("{} build dir(s) kept by --keep-days/--keep-size.", summary.kept_by_filter);
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

#[cfg(test)]
mod tests {
	use super::*;

	fn flags(keep_days: Option<u64>, keep_size: Option<u64>) -> Flags {
		Flags {
			yes: false,
			orphans: false,
			dry_run: false,
			verbose: false,
			show_size: false,
			keep_days,
			keep_size,
		}
	}

	fn measurement(bytes: u64, age_days: u64, complete: bool) -> Measurement {
		Measurement {
			bytes,
			newest: SystemTime::now() - Duration::from_secs(age_days * 86_400),
			complete,
		}
	}

	#[test]
	fn no_filters_allow_everything_even_unmeasured() {
		assert!(filters_allow(None, flags(None, None)));
	}

	#[test]
	fn keep_days_protects_recent_and_allows_stale() {
		let f = flags(Some(30), None);
		assert!(!filters_allow(Some(measurement(0, 5, true)), f), "recent → kept");
		assert!(filters_allow(Some(measurement(0, 40, true)), f), "stale → cleanable");
	}

	#[test]
	fn keep_size_protects_small_and_allows_large() {
		let f = flags(None, Some(1024));
		assert!(!filters_allow(Some(measurement(512, 99, true)), f), "small → kept");
		assert!(filters_allow(Some(measurement(4096, 99, true)), f), "large → cleanable");
	}

	#[test]
	fn an_incomplete_measurement_is_always_kept() {
		// An unreadable subtree could be arbitrarily recent or large; with a filter
		// set we must never clean what we failed to fully inspect.
		let stale_and_large = measurement(1 << 30, 400, false);
		assert!(!filters_allow(Some(stale_and_large), flags(Some(30), None)));
		assert!(!filters_allow(Some(stale_and_large), flags(None, Some(1024))));
	}
}
