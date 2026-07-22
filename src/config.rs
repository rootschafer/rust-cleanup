//! The optional config file at `~/.config/rust-cleanup/config.toml`. Every key
//! is optional and mirrors a CLI flag; a command-line flag always wins over the
//! config, which in turn wins over the built-in default. When the file is absent
//! — the common case — behavior is exactly as if it didn't exist.

use std::{env, path::PathBuf};

use serde::Deserialize;

/// The config file's shape. `None`/empty means "not set here, fall through to the
/// built-in default". Unknown keys are rejected so a typo can't silently do
/// nothing.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
	pub(crate) path: Option<PathBuf>,
	pub(crate) follow_symlinks: Option<bool>,
	pub(crate) max_depth: Option<usize>,
	pub(crate) orphans: Option<bool>,
	pub(crate) dry_run: Option<bool>,
	pub(crate) verbose: Option<bool>,
	pub(crate) show_size: Option<bool>,
	pub(crate) yes_cargo: Option<bool>,
	pub(crate) yes_dioxus: Option<bool>,
	pub(crate) yes_all: Option<bool>,
	pub(crate) keep_days: Option<u64>,
	/// A human size like `500MB`; parsed with `util::parse_size` when merging.
	pub(crate) keep_size: Option<String>,
	/// Directory trees never scanned (a path prefix match, so descendants too).
	pub(crate) ignore_paths: Vec<PathBuf>,
	/// Directory names never descended into, anywhere in the tree.
	pub(crate) ignore_names: Vec<String>,
}

impl Config {
	/// Whether the config file is what turns on auto-cleaning, so a run can say so
	/// rather than deleting without a prompt for reasons the command line doesn't
	/// show.
	pub(crate) fn sets_autoclean(&self) -> bool {
		[self.yes_all, self.yes_cargo, self.yes_dioxus].contains(&Some(true))
	}
}

/// Where the config lives: `$RUST_CLEANUP_CONFIG` if set (an escape hatch, and
/// what the tests use), else `$XDG_CONFIG_HOME/rust-cleanup/config.toml`, else
/// `~/.config/rust-cleanup/config.toml`. `None` when no home directory can be
/// determined at all.
pub(crate) fn config_path() -> Option<PathBuf> {
	if let Some(explicit) = env::var_os("RUST_CLEANUP_CONFIG") {
		return Some(PathBuf::from(explicit));
	}
	let base = env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.filter(|p| p.is_absolute())
		.or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
		.or_else(|| env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".config")))?;
	Some(base.join("rust-cleanup").join("config.toml"))
}

/// Loads the config. A missing file is the normal case and yields defaults. A
/// present-but-broken file warns (naming the file and the error) and yields
/// defaults too: a typo should never silently change what we delete, but it also
/// shouldn't stop the run.
pub(crate) fn load() -> Config {
	let Some(path) = config_path() else {
		return Config::default();
	};
	let text = match std::fs::read_to_string(&path) {
		Ok(text) => text,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
		Err(e) => {
			eprintln!("Warning: couldn't read {}: {e}; using defaults.", path.display());
			return Config::default();
		}
	};
	parse(&text).unwrap_or_else(|e| {
		eprintln!("Warning: invalid config at {}: {e}; using defaults.", path.display());
		Config::default()
	})
}

fn parse(text: &str) -> Result<Config, toml::de::Error> {
	toml::from_str(text)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_a_full_config() {
		let cfg = parse(
			r#"
			path = "/tmp/code"
			follow_symlinks = true
			max_depth = 3
			orphans = true
			show_size = true
			keep_days = 14
			keep_size = "500MiB"
			ignore_paths = ["/tmp/skip"]
			ignore_names = ["vendor"]
			"#,
		)
		.unwrap();

		assert_eq!(cfg.path, Some(PathBuf::from("/tmp/code")));
		assert_eq!(cfg.follow_symlinks, Some(true));
		assert_eq!(cfg.max_depth, Some(3));
		assert_eq!(cfg.orphans, Some(true));
		assert_eq!(cfg.keep_days, Some(14));
		assert_eq!(cfg.keep_size.as_deref(), Some("500MiB"));
		assert_eq!(cfg.ignore_paths, vec![PathBuf::from("/tmp/skip")]);
		assert_eq!(cfg.ignore_names, vec!["vendor".to_string()]);
	}

	#[test]
	fn empty_config_sets_nothing() {
		let cfg = parse("").unwrap();

		assert!(cfg.path.is_none());
		assert!(cfg.orphans.is_none());
		assert!(cfg.keep_size.is_none());
		assert!(cfg.ignore_paths.is_empty());
		assert!(cfg.ignore_names.is_empty());
		assert!(!cfg.sets_autoclean());
	}

	#[test]
	fn a_typo_is_rejected() {
		assert!(parse("orphan = true").is_err(), "unknown key must not be ignored");
		assert!(parse("max_depth = \"deep\"").is_err(), "wrong type must be an error");
	}

	#[test]
	fn autoclean_is_detected() {
		assert!(parse("yes_all = true").unwrap().sets_autoclean());
		assert!(parse("yes_dioxus = true").unwrap().sets_autoclean());
		assert!(!parse("yes_all = false").unwrap().sets_autoclean());
	}
}
