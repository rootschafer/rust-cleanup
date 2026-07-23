//! Frees disk space by cleaning the build artifacts of Rust projects under a
//! directory — including the stray build directories `cargo clean` can't reach.
//!
//! This crate is the internals of the `rustsweep` **command-line tool**, not a
//! general-purpose library: it's split into a lib so the integration suite can
//! drive the same code the binary runs. The only public entry point is
//! [`cli::run_cli`], which parses arguments and executes the whole pipeline.
//! If you're looking to *use* rustsweep, the command-line tool and its
//! documentation are what you want:
//!
//! - **User guide:** <https://rootschafer.github.io/rustsweep/>
//! - **Source & issues:** <https://github.com/rootschafer/rustsweep>
//!
//! # How it works
//!
//! A run is three stages, each feeding the next:
//!
//! 1. `discover` walks the tree once, in parallel, collecting every Cargo
//!    project and every cargo-authored build directory. A directory counts as a
//!    build directory only if it holds a `CACHEDIR.TAG` whose body mentions
//!    cargo — which is what lets a renamed or relocated `target/` be found
//!    without ever matching an unrelated directory named `target`.
//! 2. `plan` asks `cargo metadata` for each project's authoritative workspace
//!    root and build directory, folding workspace members into a single job.
//! 3. `clean` executes the plan: `cargo clean` per project, direct removal for
//!    the strays cargo can't reach, all funneled through one deletion path.
//!
//! (Those three stages are private modules — the pipeline is an implementation
//! detail of the binary, not a supported API.)

pub mod cli;

mod clean;
mod config;
mod discover;
mod ignore;
mod plan;
mod util;
