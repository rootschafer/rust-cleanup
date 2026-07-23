use std::{
	collections::HashSet,
	fs,
	path::{Path, PathBuf},
	sync::{
		Mutex,
		atomic::{AtomicUsize, Ordering},
	},
	time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::ignore::IgnoreSet;

/// The cache-directory tag cargo drops into every build directory. Its body
/// contains the word "cargo" (`... created by cargo.`), which lets us tell a
/// Cargo build dir apart both from an unrelated directory merely named `target`
/// and from other tools' generic `CACHEDIR.TAG` caches. We only ever delete a
/// directory that carries this cargo-authored tag.
const CACHEDIR_TAG: &str = "CACHEDIR.TAG";
const CARGO_MANIFEST: &str = "Cargo.toml";

/// How the tree is traversed.
pub(crate) struct WalkOptions {
	/// Follow symlinked directories (off by default).
	pub(crate) follow_symlinks: bool,
	/// Maximum number of directory levels below the search root to descend.
	pub(crate) max_depth: Option<usize>,
	/// Directories never descended into.
	pub(crate) ignore: IgnoreSet,
}

pub(crate) struct Discovery {
	/// Every directory holding a `Cargo.toml`, canonicalized.
	pub(crate) projects: Vec<PathBuf>,
	/// Every cargo-authored build directory found, canonicalized.
	pub(crate) build_dirs: Vec<PathBuf>,
}

/// Walks the tree under `start` once, in parallel, collecting every Cargo
/// project and every cargo-authored build directory. Prunes ignored directories
/// and never descends into a cache directory (`CACHEDIR.TAG`).
///
/// `start` itself is deliberately never tested against the ignore list — only
/// the directories below it are. Pointing `--path` straight at an ignored
/// directory is an explicit request, and it beats the config.
pub(crate) fn discover(start: &Path, options: WalkOptions) -> Discovery {
	let spinner = ProgressBar::new_spinner();
	spinner.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
	spinner.enable_steady_tick(Duration::from_millis(100));
	spinner.set_message("Scanning for Rust projects…");

	let ctx = WalkCtx {
		follow_symlinks: options.follow_symlinks,
		max_depth: options.max_depth,
		ignore: options.ignore,
		// The cycle guard is only needed (and only paid for) when following symlinks;
		// without them a directory tree can't contain a cycle.
		visited: options.follow_symlinks.then(|| Mutex::new(HashSet::new())),
		scanned: AtomicUsize::new(0),
	};
	let WalkResult { projects, build_dirs } = walk(start, 0, &ctx);

	spinner.finish_and_clear();
	println!(
		"Scanned {} directories: found {} project(s) and {} build dir(s).",
		ctx.scanned.load(Ordering::Relaxed),
		projects.len(),
		build_dirs.len(),
	);

	Discovery { projects, build_dirs }
}

struct WalkCtx {
	follow_symlinks: bool,
	max_depth: Option<usize>,
	ignore: IgnoreSet,
	/// Canonical paths already visited, guarding against symlink cycles. `None`
	/// when not following symlinks.
	visited: Option<Mutex<HashSet<PathBuf>>>,
	scanned: AtomicUsize,
}

#[derive(Default)]
struct WalkResult {
	projects: Vec<PathBuf>,
	build_dirs: Vec<PathBuf>,
}

impl WalkResult {
	fn merge(mut self, other: WalkResult) -> WalkResult {
		self.projects.extend(other.projects);
		self.build_dirs.extend(other.build_dirs);
		self
	}
}

/// Reads one directory, classifies it, and recurses into its (non-ignored)
/// subdirectories in parallel. Because we already hold the directory listing, we
/// detect the `Cargo.toml`/`CACHEDIR.TAG` markers from the entry names — no
/// extra `stat` per candidate.
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
	let mut subdirs: Vec<PathBuf> = Vec::new();

	for entry in entries.flatten() {
		let Ok(file_type) = entry.file_type() else {
			continue;
		};
		let name = entry.file_name();
		let name = name.to_str();

		// `file_type` is not symlink-followed. A symlinked directory only counts as
		// a directory to recurse into when --follow-symlinks is set.
		let is_dir = file_type.is_dir() || (ctx.follow_symlinks && file_type.is_symlink() && entry.path().is_dir());

		if is_dir {
			let path = entry.path();
			if !ctx.ignore.matches(&path, name) {
				subdirs.push(path);
			}
		} else if let Some(name) = name {
			match name {
				CACHEDIR_TAG => has_tag = true,
				CARGO_MANIFEST => has_cargo = true,
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

	if has_cargo && let Ok(canon) = dir.canonicalize() {
		result.projects.push(canon);
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
pub(crate) fn containing_project<'a>(build_dir: &Path, projects: &'a [PathBuf]) -> Option<&'a PathBuf> {
	projects
		.iter()
		.filter(|dir| build_dir.starts_with(dir) && build_dir != dir.as_path())
		.max_by_key(|dir| dir.components().count())
}
