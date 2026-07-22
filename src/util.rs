use std::path::{Path, PathBuf};

/// Canonicalizes `path`, falling back to the path as-is when it can't be
/// resolved (e.g. it no longer exists). Canonical form is what lets us compare
/// paths from different sources (the walk, `cargo metadata`) reliably.
pub(crate) fn canonical_or(path: &Path) -> PathBuf {
	path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Expands a leading `~` (alone or as `~/…`) to `$HOME`. Only the config file
/// needs this: the shell already expands `~` in command-line arguments, but TOML
/// does not. A `~user` form is left untouched — we don't resolve other users.
pub(crate) fn expand_tilde(path: PathBuf) -> PathBuf {
	let Some(rest) = path.to_str().and_then(|s| s.strip_prefix('~')) else {
		return path;
	};
	if !(rest.is_empty() || rest.starts_with('/')) {
		return path; // `~user/...` — not ours to expand
	}
	let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
		return path;
	};
	PathBuf::from(home).join(rest.trim_start_matches('/'))
}

/// Parses a human size like `500MB`, `2G`, `1GiB`, or a raw byte count. Units are
/// 1024-based; a trailing `b`/`ib` is optional (`GB`, `GiB`, and `G` are equal).
pub(crate) fn parse_size(s: &str) -> Result<u64, String> {
	let lower = s.trim().to_ascii_lowercase();
	let core = lower
		.strip_suffix("ib")
		.or_else(|| lower.strip_suffix('b'))
		.unwrap_or(&lower);
	let (digits, mult): (&str, u64) = match core.strip_suffix('k') {
		Some(d) => (d, 1 << 10),
		None => match core.strip_suffix('m') {
			Some(d) => (d, 1 << 20),
			None => match core.strip_suffix('g') {
				Some(d) => (d, 1 << 30),
				None => match core.strip_suffix('t') {
					Some(d) => (d, 1 << 40),
					None => (core, 1),
				},
			},
		},
	};
	let value: f64 = digits
		.trim()
		.parse()
		.map_err(|_| format!("invalid size '{s}' (try e.g. 500MB, 1GiB)"))?;
	if value < 0.0 {
		return Err(format!("size can't be negative: '{s}'"));
	}
	Ok((value * mult as f64) as u64)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn expands_a_leading_tilde() {
		let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set in tests"));

		assert_eq!(expand_tilde(PathBuf::from("~/Code")), home.join("Code"));
		assert_eq!(expand_tilde(PathBuf::from("~")), home);
		assert_eq!(
			expand_tilde(PathBuf::from("/abs/~/x")),
			PathBuf::from("/abs/~/x"),
			"only a LEADING tilde is expanded"
		);
		assert_eq!(
			expand_tilde(PathBuf::from("~other/x")),
			PathBuf::from("~other/x"),
			"another user's home is left alone"
		);
	}

	#[test]
	fn parses_human_sizes() {
		assert_eq!(parse_size("1024").unwrap(), 1024);
		assert_eq!(parse_size("1k").unwrap(), 1024);
		assert_eq!(parse_size("1KB").unwrap(), 1024);
		assert_eq!(parse_size("1KiB").unwrap(), 1024);
		assert_eq!(parse_size("2M").unwrap(), 2 << 20);
		assert_eq!(parse_size("1GB").unwrap(), 1 << 30);
		assert_eq!(parse_size("1gib").unwrap(), 1 << 30);
		assert_eq!(parse_size(" 3 g ").unwrap(), 3u64 << 30);
		assert_eq!(parse_size("1.5G").unwrap(), (1.5 * (1u64 << 30) as f64) as u64);
		assert!(parse_size("abc").is_err());
		assert!(parse_size("-5M").is_err());
	}
}
