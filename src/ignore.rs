//! The ignore list: directories the walk never descends into, so they're never
//! scanned and never cleaned. Patterns come from the config file's `ignore` key
//! and from `--ignore`, and follow `.gitignore`-style rules:
//!
//! - A bare name (`vendor`) matches a directory with that name at any depth.
//! - A pattern containing `/` or a glob metacharacter is a glob matched against
//!   the directory's absolute path (`~/Code/*/vendor`, `/opt/toolchains/**`).
//! - A leading `~` is expanded, and `./x` is resolved against the current dir.
//!
//! Bare literal names are kept in a `HashSet` rather than the glob set: the
//! always-on built-ins (`.git`, `node_modules`, …) live there, so an ordinary
//! run compares file names and never pays for a path canonicalization. Only a
//! user-supplied glob turns that on.

use std::{
	collections::HashSet,
	path::{Path, PathBuf},
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::util::{canonical_or, expand_tilde};

/// Directory names never descended into: large, noisy trees that never hold a
/// Cargo project. (Build directories are pruned dynamically via `CACHEDIR.TAG`,
/// so `target` is intentionally not listed here — it might have been renamed.)
const BUILT_IN: [&str; 3] = [".git", "node_modules", ".jj"];

pub(crate) struct IgnoreSet {
	/// Literal directory names, matched against a directory's own name.
	names: HashSet<String>,
	/// Everything else, matched against a directory's absolute path.
	globs: GlobSet,
	/// Whether `globs` holds anything — when it doesn't, matching is name-only
	/// and costs no filesystem work.
	has_globs: bool,
}

impl IgnoreSet {
	/// Compiles `patterns` on top of the built-in names. An unparseable pattern
	/// warns and is dropped: the rest of the list should still protect what it
	/// names. The built-ins can be added to but never removed.
	pub(crate) fn build(patterns: &[String]) -> IgnoreSet {
		let mut names: HashSet<String> = BUILT_IN.iter().map(|n| (*n).to_string()).collect();
		let mut builder = GlobSetBuilder::new();
		let mut has_globs = false;

		for pattern in patterns {
			let pattern = pattern.trim();
			if pattern.is_empty() {
				continue;
			}
			if is_bare_name(pattern) {
				names.insert(pattern.to_string());
				continue;
			}
			// A directory pattern should cover the directory itself and everything under
			// it, so that ignoring `/a/b` still holds when the scan starts inside it.
			let base = absolute_pattern(pattern);
			let under = format!("{}/**", base.trim_end_matches('/'));
			for glob in [base.clone(), under] {
				// `literal_separator` is what makes `*` stop at a path separator, so
				// `/home/*/scratch` means one level down — the `.gitignore` reading of it.
				// `**` still spans components.
				match GlobBuilder::new(&glob).literal_separator(true).build() {
					Ok(g) => {
						builder.add(g);
						has_globs = true;
					}
					Err(e) => eprintln!("Warning: ignoring unusable ignore pattern '{pattern}': {e}"),
				}
			}
		}

		// A GlobSetBuilder only fails on a glob it already accepted individually.
		let globs = builder.build().unwrap_or_else(|_| GlobSet::empty());
		IgnoreSet { names, globs, has_globs }
	}

	/// Whether a directory should be skipped. `name` is the directory's own file
	/// name; `path` is only touched (and canonicalized) when a glob is configured.
	///
	/// Both the path as walked and its canonical form are tried, because only one
	/// of them may look like what the user wrote: on macOS `/var/…` canonicalizes
	/// to `/private/var/…`, and a walk rooted at a relative `--path` yields
	/// relative paths that no absolute pattern could match.
	pub(crate) fn matches(&self, path: &Path, name: Option<&str>) -> bool {
		if name.is_some_and(|n| self.names.contains(n)) {
			return true;
		}
		self.has_globs && (self.globs.is_match(path) || self.globs.is_match(canonical_or(path)))
	}
}

/// A pattern with no separator and no glob syntax names a directory, and is
/// matched by name at any depth — the `.gitignore` rule.
fn is_bare_name(pattern: &str) -> bool {
	!pattern.contains('/') && !pattern.contains(['*', '?', '[', ']', '{', '}'])
}

/// Anchors a pattern to an absolute path, since that's what it gets matched
/// against: `~` expands, `./x` and `../x` resolve against the current directory,
/// and anything else stays relative to *any* ancestor (`a/b` → `**/a/b`).
fn absolute_pattern(pattern: &str) -> String {
	let expanded = expand_tilde(PathBuf::from(pattern));
	if expanded.is_absolute() {
		// A plain path (no glob syntax) is canonicalized, so a pattern written
		// through a symlink still matches the real directory the walk reports.
		if !pattern.contains(['*', '?', '[', ']', '{', '}']) {
			return canonical_or(&expanded).to_string_lossy().into_owned();
		}
		return expanded.to_string_lossy().into_owned();
	}
	let text = expanded.to_string_lossy().into_owned();
	if text.starts_with("./") || text.starts_with("../") {
		return std::env::current_dir()
			.map(|cwd| cwd.join(&text).to_string_lossy().into_owned())
			.unwrap_or(text);
	}
	format!("**/{text}")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ignores(patterns: &[&str], path: &str) -> bool {
		let patterns: Vec<String> = patterns.iter().map(|p| (*p).to_string()).collect();
		let path = Path::new(path);
		IgnoreSet::build(&patterns).matches(path, path.file_name().and_then(|n| n.to_str()))
	}

	#[test]
	fn built_ins_are_always_ignored() {
		assert!(ignores(&[], "/home/me/code/.git"));
		assert!(ignores(&[], "/home/me/code/node_modules"));
		assert!(!ignores(&[], "/home/me/code/src"));
	}

	#[test]
	fn a_bare_name_matches_at_any_depth() {
		assert!(ignores(&["vendor"], "/home/me/a/b/vendor"));
		assert!(ignores(&["vendor"], "/vendor"));
		assert!(!ignores(&["vendor"], "/home/me/vendored"));
	}

	#[test]
	fn an_absolute_pattern_matches_the_dir_and_its_contents() {
		assert!(ignores(&["/opt/toolchains"], "/opt/toolchains"));
		assert!(ignores(&["/opt/toolchains"], "/opt/toolchains/esp/xtensa"));
		assert!(!ignores(&["/opt/toolchains"], "/opt/other"));
	}

	#[test]
	fn globs_are_supported() {
		assert!(ignores(&["/home/*/scratch"], "/home/me/scratch"));
		assert!(ignores(&["**/generated/*"], "/a/generated/x"));
		assert!(ignores(&["a/b"], "/home/me/a/b"), "a relative pattern matches any ancestor");
		assert!(!ignores(&["/home/*/scratch"], "/home/me/deep/scratch"));
	}

	#[test]
	fn a_broken_pattern_does_not_sink_the_rest() {
		let set = IgnoreSet::build(&["/a/[".to_string(), "vendor".to_string()]);
		assert!(set.matches(Path::new("/x/vendor"), Some("vendor")));
	}

	#[test]
	fn name_only_lists_never_touch_the_filesystem() {
		assert!(
			!IgnoreSet::build(&["vendor".to_string()]).has_globs,
			"bare names must stay off the glob path so ordinary runs pay nothing"
		);
		assert!(IgnoreSet::build(&["/a/b".to_string()]).has_globs);
	}
}
