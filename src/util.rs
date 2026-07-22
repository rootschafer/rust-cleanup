use std::path::{Path, PathBuf};

/// Canonicalizes `path`, falling back to the path as-is when it can't be
/// resolved (e.g. it no longer exists). Canonical form is what lets us compare
/// paths from different sources (the walk, `cargo metadata`) reliably.
pub(crate) fn canonical_or(path: &Path) -> PathBuf {
	path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
