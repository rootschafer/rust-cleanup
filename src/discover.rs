use std::{
	fs,
	path::{Path, PathBuf},
	sync::atomic::{AtomicUsize, Ordering},
	time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::cli::Flags;

/// The cache-directory tag cargo drops into every build directory. Its body
/// contains the word "cargo" (`... created by cargo.`), which lets us tell a
/// Cargo build dir apart both from an unrelated directory merely named `target`
/// and from other tools' generic `CACHEDIR.TAG` caches. We only ever delete a
/// directory that carries this cargo-authored tag.
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";
const CARGO_MANIFEST: &str = "Cargo.toml";
const DIOXUS_MANIFEST: &str = "Dioxus.toml";

/// Directory names we never descend into: large, noisy trees that never hold a
/// Cargo project. (Build directories are pruned dynamically via `CACHEDIR_TAG`,
/// so `target` is intentionally not listed here — it might have been renamed.)
const PRUNED_DIRS: [&str; 3] = [".git", "node_modules", ".jj"];

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum ProjectType {
	Rust,
	Dioxus,
}

impl ProjectType {
	pub(crate) fn display_name(self) -> &'static str {
		match self {
			Self::Rust => "Rust",
			Self::Dioxus => "Dioxus",
		}
	}

	pub(crate) fn should_autoclean(self, flags: Flags) -> bool {
		flags.yes_all
			|| match self {
				Self::Rust => flags.yes_cargo,
				Self::Dioxus => flags.yes_dioxus,
			}
	}
}

pub(crate) struct Candidate {
	pub(crate) dir: PathBuf,
	pub(crate) kind: ProjectType,
}

pub(crate) struct Discovery {
	pub(crate) candidates: Vec<Candidate>,
	/// Every cargo-authored build directory found, canonicalized.
	pub(crate) build_dirs: Vec<PathBuf>,
}

/// Walks the tree under `start` once, in parallel, collecting every Cargo
/// project and every cargo-authored build directory. Prunes VCS/dependency
/// trees by name and never descends into a cache directory (`CACHEDIR.TAG`).
pub(crate) fn discover(start: &Path) -> Discovery {
	let spinner = ProgressBar::new_spinner();
	spinner.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
	spinner.enable_steady_tick(Duration::from_millis(100));
	spinner.set_message("Scanning for Rust projects…");

	let scanned = AtomicUsize::new(0);
	let WalkResult {
		candidates,
		build_dirs,
	} = walk(start, &scanned);

	spinner.finish_and_clear();
	println!(
		"Scanned {} directories: found {} project(s) and {} build dir(s).",
		scanned.load(Ordering::Relaxed),
		candidates.len(),
		build_dirs.len(),
	);

	Discovery {
		candidates,
		build_dirs,
	}
}

#[derive(Default)]
struct WalkResult {
	candidates: Vec<Candidate>,
	build_dirs: Vec<PathBuf>,
}

impl WalkResult {
	fn merge(mut self, other: WalkResult) -> WalkResult {
		self.candidates.extend(other.candidates);
		self.build_dirs.extend(other.build_dirs);
		self
	}
}

/// Reads one directory, classifies it, and recurses into its (non-pruned)
/// subdirectories in parallel. Because we already hold the directory listing, we
/// detect the `Cargo.toml`/`Dioxus.toml`/`CACHEDIR.TAG` markers from the entry
/// names — no extra `stat` per candidate.
fn walk(dir: &Path, scanned: &AtomicUsize) -> WalkResult {
	scanned.fetch_add(1, Ordering::Relaxed);
	let mut result = WalkResult::default();

	let Ok(entries) = fs::read_dir(dir) else {
		return result; // unreadable (permissions, races) — skip quietly
	};

	let mut has_tag = false;
	let mut has_cargo = false;
	let mut has_dioxus = false;
	let mut subdirs: Vec<PathBuf> = Vec::new();

	for entry in entries.flatten() {
		let Ok(file_type) = entry.file_type() else {
			continue;
		};
		let name = entry.file_name();
		let name = name.to_str();

		// `file_type` is not symlink-followed, so symlinked dirs are treated as
		// files here and never recursed into — this is what stops walk cycles.
		if file_type.is_dir() {
			if !name.is_some_and(|n| PRUNED_DIRS.contains(&n)) {
				subdirs.push(entry.path());
			}
		} else if let Some(name) = name {
			match name {
				CACHEDIR_TAG => has_tag = true,
				CARGO_MANIFEST => has_cargo = true,
				DIOXUS_MANIFEST => has_dioxus = true,
				_ => {}
			}
		}
	}

	// A cache directory holds no projects, so we prune it. If cargo authored the
	// tag, it's a build dir worth recording.
	if has_tag {
		if is_cargo_build_dir(dir)
			&& let Ok(canon) = dir.canonicalize()
		{
			result.build_dirs.push(canon);
		}
		return result;
	}

	if has_cargo {
		let kind = if has_dioxus {
			ProjectType::Dioxus
		} else {
			ProjectType::Rust
		};
		if let Ok(canon) = dir.canonicalize() {
			result.candidates.push(Candidate { dir: canon, kind });
		}
		// Keep descending: workspaces hold nested member crates, and crates can hold
		// their own nested crates (fuzz/, xtask/, examples that are crates, …).
	}

	if !subdirs.is_empty() {
		let children = subdirs
			.par_iter()
			.map(|sub| walk(sub, scanned))
			.reduce(WalkResult::default, WalkResult::merge);
		result = result.merge(children);
	}

	result
}

/// Whether `dir` holds a cargo-authored `CACHEDIR.TAG` (as opposed to another
/// tool's generic cache tag).
fn is_cargo_build_dir(dir: &Path) -> bool {
	fs::read_to_string(dir.join(CACHEDIR_TAG)).is_ok_and(|tag| tag.contains("cargo"))
}

/// The deepest project directory that contains `build_dir`, if any.
pub(crate) fn containing_project<'a>(
	build_dir: &Path,
	candidates: &'a [Candidate],
) -> Option<&'a Candidate> {
	candidates
		.iter()
		.filter(|c| build_dir.starts_with(&c.dir) && build_dir != c.dir.as_path())
		.max_by_key(|c| c.dir.components().count())
}
