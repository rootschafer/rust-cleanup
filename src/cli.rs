use std::path::PathBuf;

use clap::{Args, Parser};

use crate::clean;
use crate::discover::discover;
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
}

pub fn run_cli() {
	let cli = Cli::parse();

	configure_thread_pool();

	let discovery = discover(&cli.path);
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
