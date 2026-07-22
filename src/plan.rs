use std::{
	collections::HashSet,
	path::{Path, PathBuf},
	time::Duration,
};

use cargo_metadata::MetadataCommand;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::{
	discover::{Candidate, ProjectType},
	util::canonical_or,
};

/// A distinct thing to clean: one standalone crate or one workspace, together
/// with the build directory cargo resolved for it.
pub(crate) struct Workspace {
	pub(crate) root: PathBuf,
	pub(crate) target_dir: PathBuf,
	pub(crate) kind: ProjectType,
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
pub(crate) fn build_plan(candidates: &[Candidate]) -> (Vec<Workspace>, Vec<(PathBuf, String)>) {
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
	let Ok(contents) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
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
