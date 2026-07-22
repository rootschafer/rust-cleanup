use std::path::PathBuf;

use clap::parser::ValueSource;
use clap::{ArgMatches, Args, CommandFactory, FromArgMatches, Parser};

use crate::clean;
use crate::config::{self, Config};
use crate::discover::{WalkOptions, default_ignore_names, discover};
use crate::plan::build_plan;
use crate::util::{expand_tilde, parse_size};

/// Frees disk space by cleaning the build artifacts of Rust projects under a
/// directory. Already-clean projects are skipped, workspaces are cleaned once,
/// and stray/orphaned build dirs are detected by cargo's `CACHEDIR.TAG`.
#[derive(Parser)]
#[command(
	name = "rust-cleanup",
	version,
	about,
	after_help = "Defaults for these options can be set in ~/.config/rust-cleanup/config.toml \
(a command-line flag always wins). That file also holds the global ignore list; \
--ignore adds to it rather than replacing it."
)]
pub struct Cli {
	/// Sets the starting directory for the search
	#[arg(short, long, value_name = "PATH", default_value = ".")]
	pub path: PathBuf,

	/// Follow symlinked directories while searching (off by default so the search
	/// can't escape the tree; a cycle guard and --max-depth bound the traversal)
	#[arg(short = 'L', long)]
	pub follow_symlinks: bool,

	/// Limit how many directory levels below the search root to descend
	#[arg(short = 'd', long, value_name = "DEPTH")]
	pub max_depth: Option<usize>,

	/// Never scan (or clean) anything inside this directory. Repeatable; adds to
	/// the config file's ignore_paths rather than replacing it.
	#[arg(long = "ignore", value_name = "PATH")]
	pub ignore: Vec<PathBuf>,

	#[command(flatten)]
	pub flags: Flags,
}

/// The auto-clean / behavior flags, shared with the cleaning logic. Kept as a
/// small `Copy` bundle so it can be threaded through by value.
#[derive(Args, Clone, Copy)]
pub struct Flags {
	/// Automatically clean non-Dioxus Rust projects without prompting
	#[arg(long)]
	pub yes_cargo: bool,

	/// Automatically clean Dioxus projects without prompting
	#[arg(long)]
	pub yes_dioxus: bool,

	/// Automatically clean all projects without prompting for a yes or a no
	#[arg(short = 'y', long)]
	pub yes_all: bool,

	/// Also remove Cargo build dirs that aren't inside any discovered project
	/// (e.g. left over from `cargo build --target-dir <dir>`)
	#[arg(long)]
	pub orphans: bool,

	/// Show what would be cleaned without deleting anything or prompting
	#[arg(short = 'n', long)]
	pub dry_run: bool,

	/// List the projects that `cargo metadata` could not read
	#[arg(short, long)]
	pub verbose: bool,

	/// Show each build dir's size before you decide (and a freed-space total).
	/// Measuring walks every target, so this is slower than a normal run.
	#[arg(short = 's', long)]
	pub show_size: bool,

	/// Keep (don't clean) build dirs touched within the last N days; only clean
	/// ones untouched for longer. Protects projects you're actively building.
	#[arg(long, value_name = "DAYS")]
	pub keep_days: Option<u64>,

	/// Only clean build dirs at least this large; keep smaller ones. Accepts
	/// units, e.g. 500MB, 1GiB (1024-based). Measuring sizes walks each target,
	/// so this is slower than a normal run.
	#[arg(long, value_name = "SIZE", value_parser = parse_size)]
	pub keep_size: Option<u64>,
}

pub fn run_cli() {
	// Parsed via `ArgMatches` (rather than `Cli::parse()`) so we can ask clap which
	// values actually came from the command line — that's what lets the config file
	// enable a bool flag that clap would otherwise report as a plain `false`.
	let matches = Cli::command().get_matches();
	let cli = Cli::from_arg_matches(&matches).expect("clap validated the args");
	let cfg = config::load();

	configure_thread_pool();

	let resolved = resolve(cli, &matches, cfg);

	let discovery = discover(&resolved.path, resolved.walk);
	let (workspaces, failed) = build_plan(&discovery.candidates);
	clean::run(resolved.flags, &discovery, &workspaces, &failed);
}

/// The inputs after merging CLI > config > built-in default.
struct Resolved {
	path: PathBuf,
	walk: WalkOptions,
	flags: Flags,
}

/// Merges the command line over the config file. `Option`-valued settings merge
/// with `.or()`; bools consult clap's value source, since a `false` there could
/// mean either "passed" or "absent"; the ignore lists are additive (config ∪ CLI)
/// so nothing can quietly un-protect a directory.
fn resolve(cli: Cli, matches: &ArgMatches, cfg: Config) -> Resolved {
	let cfg_sets_autoclean = cfg.sets_autoclean();

	let path = if from_cli(matches, "path") {
		cli.path
	} else {
		cfg.path.map(expand_tilde).unwrap_or(cli.path)
	};

	let mut ignore_names = default_ignore_names();
	ignore_names.extend(cfg.ignore_names);

	let ignore_paths: Vec<PathBuf> = cfg
		.ignore_paths
		.into_iter()
		.map(expand_tilde)
		.chain(cli.ignore)
		.filter_map(|p| match p.canonicalize() {
			Ok(canon) => Some(canon),
			// A nonexistent ignore path protects nothing, so it's harmless — but say so,
			// since the user probably meant it to match something.
			Err(e) => {
				eprintln!("Warning: ignoring unusable ignore path {}: {e}", p.display());
				None
			}
		})
		.collect();

	let walk = WalkOptions {
		follow_symlinks: merge_bool(matches, "follow_symlinks", cli.follow_symlinks, cfg.follow_symlinks),
		max_depth: cli.max_depth.or(cfg.max_depth),
		ignore_names,
		ignore_paths,
	};

	let flags = Flags {
		yes_cargo: merge_bool(matches, "yes_cargo", cli.flags.yes_cargo, cfg.yes_cargo),
		yes_dioxus: merge_bool(matches, "yes_dioxus", cli.flags.yes_dioxus, cfg.yes_dioxus),
		yes_all: merge_bool(matches, "yes_all", cli.flags.yes_all, cfg.yes_all),
		orphans: merge_bool(matches, "orphans", cli.flags.orphans, cfg.orphans),
		dry_run: merge_bool(matches, "dry_run", cli.flags.dry_run, cfg.dry_run),
		verbose: merge_bool(matches, "verbose", cli.flags.verbose, cfg.verbose),
		show_size: merge_bool(matches, "show_size", cli.flags.show_size, cfg.show_size),
		keep_days: cli.flags.keep_days.or(cfg.keep_days),
		keep_size: cli
			.flags
			.keep_size
			.or_else(|| config_keep_size(cfg.keep_size.as_deref())),
	};

	// Auto-cleaning deletes without asking. When that came from the config rather
	// than this command line, say so — it should never be a silent surprise.
	if cfg_sets_autoclean && !flags.dry_run {
		let from_command_line = ["yes_all", "yes_cargo", "yes_dioxus"]
			.iter()
			.any(|id| from_cli(matches, id));
		if !from_command_line {
			println!("Auto-cleaning without prompts (enabled by the config file).");
		}
	}

	Resolved { path, walk, flags }
}

/// Whether this value came from the command line, as opposed to a default clap
/// filled in.
fn from_cli(matches: &ArgMatches, id: &str) -> bool {
	matches.value_source(id) == Some(ValueSource::CommandLine)
}

/// A bool flag's effective value: the command line if it was passed there,
/// otherwise the config, otherwise clap's default.
fn merge_bool(matches: &ArgMatches, id: &str, cli: bool, cfg: Option<bool>) -> bool {
	if from_cli(matches, id) { cli } else { cfg.unwrap_or(cli) }
}

/// Parses the config's `keep_size` string. A bad value warns and is treated as
/// unset rather than aborting the run (same policy as the rest of the config).
fn config_keep_size(raw: Option<&str>) -> Option<u64> {
	match parse_size(raw?) {
		Ok(bytes) => Some(bytes),
		Err(e) => {
			eprintln!("Warning: ignoring keep_size in config: {e}");
			None
		}
	}
}

/// Sizes rayon's global pool for our workload. The walk and the `cargo metadata`
/// calls are I/O/subprocess-bound, so we oversubscribe the core count to keep
/// more of them in flight. Skips this if the user pinned `RAYON_NUM_THREADS`, and
/// ignores the error if a pool already exists.
fn configure_thread_pool() {
	if std::env::var_os("RAYON_NUM_THREADS").is_some() {
		return;
	}
	let threads = std::thread::available_parallelism()
		.map_or(8, |n| n.get())
		.saturating_mul(2)
		.clamp(4, 32);
	let _ = rayon::ThreadPoolBuilder::new()
		.num_threads(threads)
		.build_global();
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The merge helpers address args by their clap id, which for a flattened
	/// `Args` struct is the Rust field name. If that ever changes, precedence would
	/// silently break — so assert the ids exist and resolve.
	#[test]
	fn flag_ids_are_the_field_names() {
		let matches = Cli::command()
			.get_matches_from(["rust-cleanup", "--orphans", "--show-size"]);

		for id in [
			"path",
			"follow_symlinks",
			"max_depth",
			"ignore",
			"yes_cargo",
			"yes_dioxus",
			"yes_all",
			"orphans",
			"dry_run",
			"verbose",
			"show_size",
			"keep_days",
			"keep_size",
		] {
			assert!(
				matches.try_get_one::<bool>(id).is_ok() || matches.ids().any(|i| i.as_str() == id),
				"unknown arg id: {id}"
			);
		}
		assert!(from_cli(&matches, "orphans"), "--orphans came from the command line");
		assert!(!from_cli(&matches, "verbose"), "--verbose was not passed");
		assert!(!from_cli(&matches, "path"), "--path is clap's default here");
	}

	#[test]
	fn config_fills_in_flags_the_cli_left_alone() {
		let matches = Cli::command().get_matches_from(["rust-cleanup", "--orphans"]);

		assert!(merge_bool(&matches, "orphans", true, Some(false)), "CLI wins over config");
		assert!(merge_bool(&matches, "verbose", false, Some(true)), "config enables it");
		assert!(!merge_bool(&matches, "verbose", false, None), "default stays off");
	}
}
