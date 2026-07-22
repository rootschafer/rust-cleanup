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

	let mut skipped: Vec<PathBuf> = Vec::new();
	let mut already_clean = 0usize;
	let mut kept_by_filter = 0usize;
	let mut freed = 0u64; // estimated bytes cleaned (only meaningful with --show-size)

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
		let measured = measure_if_needed(&ws.target_dir, flags);
		if !filters_allow(measured, flags) {
			kept_by_filter += 1;
			continue;
		}
		let size = measured.map_or(0, |(bytes, _)| bytes);

		if flags.dry_run {
			println!(
				"Would clean {} ({} project) — build dir {}",
				ws.root.display(),
				ws.kind.display_name(),
				with_size(&ws.target_dir, measured, flags),
			);
			freed += size;
			continue;
		}

		let question = format!(
			"{} is a {} project (build dir: {}). Clean it?",
			ws.root.display(),
			ws.kind.display_name(),
			with_size(&ws.target_dir, measured, flags),
		);
		if ws.kind.should_autoclean(flags) || prompt(&question) {
			cargo_clean(&ws.root);
			freed += size;
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
				let measured = measure_if_needed(build_dir, flags);
				if !filters_allow(measured, flags) {
					kept_by_filter += 1;
					continue;
				}
				let size = measured.map_or(0, |(bytes, _)| bytes);
				if flags.dry_run {
					println!(
						"Would remove stray build dir {} (inside {})",
						with_size(build_dir, measured, flags),
						project.dir.display(),
					);
					freed += size;
					continue;
				}
				let question = format!(
					"{} is a stray Cargo build dir (not {}'s current build dir). Remove it?",
					with_size(build_dir, measured, flags),
					project.dir.display(),
				);
				if project.kind.should_autoclean(flags) || prompt(&question) {
					remove_dir(build_dir);
					freed += size;
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
				let measured = measure_if_needed(build_dir, flags);
				if !filters_allow(measured, flags) {
					kept_by_filter += 1;
					continue;
				}
				let size = measured.map_or(0, |(bytes, _)| bytes);
				if flags.dry_run {
					println!("Would remove orphaned build dir {}", with_size(build_dir, measured, flags),);
					freed += size;
					continue;
				}
				let question = format!(
					"{} is an orphaned Cargo build dir with no associated project. Remove it?",
					with_size(build_dir, measured, flags),
				);
				if flags.yes_all || prompt(&question) {
					remove_dir(build_dir);
					freed += size;
				} else {
					skipped.push(build_dir.clone());
				}
			}
		}
	}

	if flags.show_size {
		let verb = if flags.dry_run { "Would free" } else { "Freed" };
		println!("{verb} ~{}.", human_size(freed));
	}

	print_summary(&Summary {
		already_clean,
		kept_by_filter,
		detached_found,
		orphans_enabled: flags.orphans,
		skipped: &skipped,
		failed,
		verbose: flags.verbose,
	});
}

/// Measures a build dir only when a filter or `--show-size` needs it; otherwise
/// returns `None` so a normal run pays nothing.
fn measure_if_needed(build_dir: &Path, flags: Flags) -> Option<(u64, SystemTime)> {
	if flags.keep_days.is_some() || flags.keep_size.is_some() || flags.show_size {
		measure(build_dir)
	} else {
		None
	}
}

/// Renders a build-dir path, appending its human size in parentheses when
/// `--show-size` is on and the size is known.
fn with_size(path: &Path, measured: Option<(u64, SystemTime)>, flags: Flags) -> String {
	match (flags.show_size, measured) {
		(true, Some((bytes, _))) => format!("{} ({})", path.display(), human_size(bytes)),
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
/// ones. A filter with no measurement (couldn't read it) keeps it, so we never
/// delete something we failed to inspect.
fn filters_allow(measured: Option<(u64, SystemTime)>, flags: Flags) -> bool {
	if flags.keep_days.is_none() && flags.keep_size.is_none() {
		return true;
	}
	let Some((size, newest)) = measured else {
		return false;
	};
	if let Some(days) = flags.keep_days
		&& touched_within(newest, days)
	{
		return false; // recently built — protect active work
	}
	if let Some(min_size) = flags.keep_size
		&& size < min_size
	{
		return false; // too small to be worth reclaiming
	}
	true
}

/// Total size in bytes and newest file mtime under `dir`. Walks the whole tree
/// (the inherent cost of size/age filtering); does not follow symlinks.
fn measure(dir: &Path) -> Option<(u64, SystemTime)> {
	let mut total = 0u64;
	let mut newest = SystemTime::UNIX_EPOCH;
	let mut stack = vec![dir.to_path_buf()];

	while let Some(current) = stack.pop() {
		let Ok(entries) = fs::read_dir(&current) else {
			continue;
		};
		for entry in entries.flatten() {
			let Ok(file_type) = entry.file_type() else {
				continue;
			};
			if file_type.is_dir() {
				stack.push(entry.path());
			} else if file_type.is_file()
				&& let Ok(meta) = entry.metadata()
			{
				total += meta.len();
				if let Ok(modified) = meta.modified()
					&& modified > newest
				{
					newest = modified;
				}
			}
		}
	}

	Some((total, newest))
}

/// Whether `mtime` is within the last `days` days (clock skew into the future is
/// treated as "recent").
fn touched_within(mtime: SystemTime, days: u64) -> bool {
	match SystemTime::now().duration_since(mtime) {
		Ok(elapsed) => elapsed < Duration::from_secs(days.saturating_mul(86_400)),
		Err(_) => true,
	}
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
		println!("{} build dir(s) kept by --keep-days/--keep-size.", summary.kept_by_filter,);
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
