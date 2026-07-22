use std::path::PathBuf;

use clap::{Args, Parser};

use crate::clean;
use crate::discover::{WalkOptions, discover};
use crate::plan::build_plan;

/// Frees disk space by cleaning the build artifacts of Rust projects under a
/// directory. Already-clean projects are skipped, workspaces are cleaned once,
/// and stray/orphaned build dirs are detected by cargo's `CACHEDIR.TAG`.
#[derive(Parser)]
#[command(name = "rust-cleanup", version, about)]
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

/// Parses a human size like `500MB`, `2G`, `1GiB`, or a raw byte count. Units are
/// 1024-based; a trailing `b`/`ib` is optional (`GB`, `GiB`, and `G` are equal).
fn parse_size(s: &str) -> Result<u64, String> {
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

pub fn run_cli() {
	let cli = Cli::parse();

	configure_thread_pool();

	let discovery = discover(
		&cli.path,
		WalkOptions {
			follow_symlinks: cli.follow_symlinks,
			max_depth: cli.max_depth,
		},
	);
	let (workspaces, failed) = build_plan(&discovery.candidates);
	clean::run(cli.flags, &discovery, &workspaces, &failed);
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
	use super::parse_size;

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
