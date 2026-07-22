use std::{
	collections::HashSet,
	fs,
	path::{Path, PathBuf},
	sync::Mutex,
	sync::atomic::{AtomicUsize, Ordering},
	time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::cli::Flags;
use crate::util::canonical_or;

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

/// The built-in pruned names, as the floor that a config's `ignore_names` adds
/// to (the built-ins can be extended, never removed).
pub(crate) fn default_ignore_names() -> HashSet<String> {
	PRUNED_DIRS.iter().map(|n| (*n).to_string()).collect()
}

/// How the tree is traversed.
pub(crate) struct WalkOptions {
	/// Follow symlinked directories (off by default).
	pub(crate) follow_symlinks: bool,
	/// Maximum number of directory levels below the search root to descend.
	pub(crate) max_depth: Option<usize>,
	/// Directory names never descended into — a superset of `PRUNED_DIRS`.
	pub(crate) ignore_names: HashSet<String>,
	/// Canonicalized directory trees never descended into (prefix match, so a
	/// whole subtree is skipped). Empty in the common case.
	pub(crate) ignore_paths: Vec<PathBuf>,
}

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
pub(crate) fn discover(start: &Path, options: WalkOptions) -> Discovery {
	let spinner = ProgressBar::new_spinner();
	spinner.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
	spinner.enable_steady_tick(Duration::from_millis(100));
	spinner.set_message("Scanning for Rust projects…");

	let ctx = WalkCtx {
		follow_symlinks: options.follow_symlinks,
		max_depth: options.max_depth,
		ignore_names: options.ignore_names,
		ignore_paths: options.ignore_paths,
		// The cycle guard is only needed (and only paid for) when following symlinks;
		// without them a directory tree can't contain a cycle.
		visited: options.follow_symlinks.then(|| Mutex::new(HashSet::new())),
		scanned: AtomicUsize::new(0),
	};
	let WalkResult {
		candidates,
		build_dirs,
	} = walk(start, 0, &ctx);

	spinner.finish_and_clear();
	println!(
		"Scanned {} directories: found {} project(s) and {} build dir(s).",
		ctx.scanned.load(Ordering::Relaxed),
		candidates.len(),
		build_dirs.len(),
	);

	Discovery {
		candidates,
		build_dirs,
	}
}

struct WalkCtx {
	follow_symlinks: bool,
	max_depth: Option<usize>,
	ignore_names: HashSet<String>,
	ignore_paths: Vec<PathBuf>,
	/// Canonical paths already visited, guarding against symlink cycles. `None`
	/// when not following symlinks.
	visited: Option<Mutex<HashSet<PathBuf>>>,
	scanned: AtomicUsize,
}

impl WalkCtx {
	/// Whether `dir` is inside one of the ignored trees. Costs nothing when no
	/// ignore paths are configured; only then do we pay a canonicalize per dir.
	fn is_ignored_path(&self, dir: &Path) -> bool {
		if self.ignore_paths.is_empty() {
			return false;
		}
		let canon = canonical_or(dir);
		self.ignore_paths.iter().any(|ignored| canon.starts_with(ignored))
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
fn walk(dir: &Path, depth: usize, ctx: &WalkCtx) -> WalkResult {
	ctx.scanned.fetch_add(1, Ordering::Relaxed);
	let mut result = WalkResult::default();

	// When following symlinks, skip any real directory we've already seen so a
	// link back to an ancestor can't loop forever.
	if let Some(visited) = &ctx.visited {
		let Ok(canon) = dir.canonicalize() else {
			return result;
		};
		if !visited.lock().unwrap().insert(canon) {
			return result; // already visited via another path — symlink cycle
		}
	}

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

		// `file_type` is not symlink-followed. A symlinked directory only counts as
		// a directory to recurse into when --follow-symlinks is set.
		let is_dir = file_type.is_dir()
			|| (ctx.follow_symlinks && file_type.is_symlink() && entry.path().is_dir());

		if is_dir {
			let pruned_by_name = name.is_some_and(|n| ctx.ignore_names.contains(n));
			if !pruned_by_name && !ctx.is_ignored_path(&entry.path()) {
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

	let may_descend = ctx.max_depth.is_none_or(|max| depth < max);
	if may_descend && !subdirs.is_empty() {
		let children = subdirs
			.par_iter()
			.map(|sub| walk(sub, depth + 1, ctx))
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
