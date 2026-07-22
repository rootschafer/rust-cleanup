use std::{
	collections::HashSet,
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
	process::Command as Process,
};

use crate::cli::Flags;
use crate::discover::{Discovery, containing_project};
use crate::plan::Workspace;
use crate::util::canonical_or;

/// Executes the plan: clean each project's resolved build dir, then remove any
/// stray/orphaned build dir the scan turned up, then print a summary. Honors
/// `--dry-run` by reporting instead of deleting.
pub(crate) fn run(
	flags: Flags,
	discovery: &Discovery,
	workspaces: &[Workspace],
	failed: &[(PathBuf, String)],
) {
	if flags.dry_run {
		println!("Dry run — nothing will be deleted.");
	}

	let mut skipped: Vec<PathBuf> = Vec::new();
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
		failed,
		verbose: flags.verbose,
	});
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
