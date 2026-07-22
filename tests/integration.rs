//! End-to-end tests: build a tree of dummy Cargo projects in a temp dir, run the
//! real `rust-cleanup` binary against it, and assert on the resulting filesystem
//! state and output. Fixtures use fabricated build directories (a cargo-authored
//! `CACHEDIR.TAG`) so we don't need to actually compile anything — `cargo clean`
//! still removes a project's real target dir, and the scan removes strays.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

/// Body cargo writes into a build dir's `CACHEDIR.TAG` (contains "cargo").
const CARGO_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
	# This file is a cache directory tag created by cargo.\n";
/// A generic cache tag from some other tool — must never be treated as cargo's.
const OTHER_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
	# created by some other tool\n";

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_rust-cleanup")
}

fn tmp() -> TempDir {
	tempfile::tempdir().unwrap()
}

fn write(path: &Path, contents: &str) {
	fs::create_dir_all(path.parent().unwrap()).unwrap();
	fs::write(path, contents).unwrap();
}

/// A valid standalone crate (empty lib is enough for `cargo metadata`/`clean`).
fn make_crate(dir: &Path, name: &str) {
	write(
		&dir.join("Cargo.toml"),
		&format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
	);
	write(&dir.join("src/lib.rs"), "");
}

fn make_dioxus(dir: &Path, name: &str) {
	make_crate(dir, name);
	write(&dir.join("Dioxus.toml"), "[application]\n");
}

/// A (virtual) workspace manifest listing `members`.
fn make_workspace(dir: &Path, members: &[&str]) {
	let list = members
		.iter()
		.map(|m| format!("\"{m}\""))
		.collect::<Vec<_>>()
		.join(", ");
	write(
		&dir.join("Cargo.toml"),
		&format!("[workspace]\nresolver = \"2\"\nmembers = [{list}]\n"),
	);
}

/// Fabricate a cargo build directory so the scan recognizes it as one.
fn make_build_dir(dir: &Path) {
	write(&dir.join("CACHEDIR.TAG"), CARGO_TAG);
	write(&dir.join("debug/some-artifact"), "x");
}

fn make_other_cache(dir: &Path) {
	write(&dir.join("CACHEDIR.TAG"), OTHER_TAG);
}

fn run(root: &Path, args: &[&str]) -> Output {
	Command::new(bin())
		.arg("--path")
		.arg(root)
		.args(args)
		.stdin(Stdio::null()) // closed stdin => any prompt answers "no"
		.output()
		.unwrap()
}

fn run_input(root: &Path, args: &[&str], input: &str) -> Output {
	let mut child = Command::new(bin())
		.arg("--path")
		.arg(root)
		.args(args)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	child
		.stdin
		.take()
		.unwrap()
		.write_all(input.as_bytes())
		.unwrap();
	child.wait_with_output().unwrap()
}

fn stdout(o: &Output) -> String {
	String::from_utf8_lossy(&o.stdout).into_owned()
}

// ---------------------------------------------------------------------------

#[test]
fn already_clean_project_is_left_alone() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");

	let o = run(t.path(), &["--yes-all"]);

	assert!(o.status.success());
	assert!(stdout(&o).contains("were already clean"));
}

#[test]
fn standalone_project_target_is_cleaned() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	run(t.path(), &["--yes-all"]);

	assert!(!t.path().join("a/target").exists(), "target should be gone");
}

#[test]
fn declining_the_prompt_skips_and_keeps_target() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	let o = run_input(t.path(), &[], "n\n");

	assert!(t.path().join("a/target").exists(), "target kept on 'no'");
	assert!(stdout(&o).contains("Skipped"));
}

#[test]
fn accepting_the_prompt_cleans() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	run_input(t.path(), &[], "y\n");

	assert!(!t.path().join("a/target").exists(), "target cleaned on 'yes'");
}

#[test]
fn workspace_is_cleaned_once_at_the_root() {
	let t = tmp();
	let ws = t.path().join("ws");
	make_workspace(&ws, &["crates/foo", "crates/bar"]);
	make_crate(&ws.join("crates/foo"), "foo");
	make_crate(&ws.join("crates/bar"), "bar");
	make_build_dir(&ws.join("target")); // shared build dir at the root

	let o = run(t.path(), &["--yes-all"]);

	assert!(o.status.success());
	assert!(!ws.join("target").exists(), "workspace target cleaned");
}

#[test]
fn workspace_member_stray_target_is_removed() {
	let t = tmp();
	let ws = t.path().join("ws");
	make_workspace(&ws, &["foo"]);
	make_crate(&ws.join("foo"), "foo");
	make_build_dir(&ws.join("target")); // real shared build dir
	make_build_dir(&ws.join("foo/target")); // stray left in a member

	run(t.path(), &["--yes-all"]);

	assert!(!ws.join("target").exists(), "shared target cleaned");
	assert!(!ws.join("foo/target").exists(), "member stray removed");
}

#[test]
fn detached_child_is_skipped_quietly_but_its_build_dir_is_removed() {
	let t = tmp();
	let ws = t.path().join("ws");
	make_workspace(&ws, &["foo"]); // only foo is a member
	make_crate(&ws.join("foo"), "foo");
	make_crate(&ws.join("detached"), "detached"); // present but NOT a member
	make_build_dir(&ws.join("detached/target"));

	let o = run(t.path(), &["--yes-all", "-v"]);
	let out = stdout(&o);

	assert!(
		!ws.join("detached/target").exists(),
		"detached build dir removed by scan"
	);
	assert!(
		!out.contains("couldn't be read"),
		"detached child must not be reported as a metadata failure:\n{out}"
	);
}

#[test]
fn relocated_build_dir_and_stray_are_both_removed() {
	let t = tmp();
	let p = t.path().join("reloc");
	make_crate(&p, "reloc");
	write(&p.join(".cargo/config.toml"), "[build]\ntarget-dir = \"custom\"\n");
	make_build_dir(&p.join("custom")); // the resolved build dir (via config)
	make_build_dir(&p.join("target")); // a leftover default dir → stray

	run(t.path(), &["--yes-all"]);

	assert!(!p.join("custom").exists(), "relocated build dir cleaned");
	assert!(!p.join("target").exists(), "stray default target removed");
}

#[test]
fn renamed_build_dir_inside_project_is_removed() {
	let t = tmp();
	let p = t.path().join("a");
	make_crate(&p, "a");
	// No `target/`; only a differently-named cargo build dir left behind.
	make_build_dir(&p.join("old-target"));

	run(t.path(), &["--yes-all"]);

	assert!(!p.join("old-target").exists(), "renamed build dir removed");
}

#[test]
fn orphan_build_dir_needs_the_orphans_flag() {
	let t = tmp();
	make_build_dir(&t.path().join("loose")); // no project anywhere near it

	let o = run(t.path(), &[]);
	assert!(t.path().join("loose").exists(), "orphan kept without --orphans");
	assert!(stdout(&o).contains("orphaned"), "hint about --orphans shown");

	run(t.path(), &["--orphans", "--yes-all"]);
	assert!(!t.path().join("loose").exists(), "orphan removed with --orphans");
}

#[test]
fn non_cargo_cache_dir_is_never_removed() {
	let t = tmp();
	make_other_cache(&t.path().join("cache"));

	run(t.path(), &["--orphans", "--yes-all"]);

	assert!(
		t.path().join("cache/CACHEDIR.TAG").exists(),
		"a non-cargo CACHEDIR.TAG dir must be left alone"
	);
}

#[test]
fn broken_manifest_is_summarized_but_its_build_dir_still_removed() {
	let t = tmp();
	let p = t.path().join("broken");
	write(
		&p.join("Cargo.toml"),
		"[package]\nname = \"broken\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\
		 [features]\nx = [\"nope\"]\n", // references a non-existent dependency
	);
	write(&p.join("src/lib.rs"), "");
	make_build_dir(&p.join("target"));

	let o = run(t.path(), &["--yes-all", "-v"]);
	let out = stdout(&o);

	assert!(!p.join("target").exists(), "build dir removed via scan");
	assert!(out.contains("couldn't be read"), "failure summarized:\n{out}");
	assert!(out.contains("broken"), "verbose lists the broken path:\n{out}");
}

#[test]
fn dry_run_deletes_nothing() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));
	make_build_dir(&t.path().join("loose"));

	let o = run(t.path(), &["--dry-run", "--orphans", "--yes-all"]);
	let out = stdout(&o);

	assert!(t.path().join("a/target").exists(), "dry run kept target");
	assert!(t.path().join("loose").exists(), "dry run kept orphan");
	assert!(out.contains("Would clean"));
	assert!(out.contains("Would remove orphaned"));
}

#[test]
fn yes_cargo_cleans_rust_but_not_dioxus() {
	let t = tmp();
	make_crate(&t.path().join("rs"), "rs");
	make_build_dir(&t.path().join("rs/target"));
	make_dioxus(&t.path().join("dx"), "dx");
	make_build_dir(&t.path().join("dx/target"));

	// --yes-cargo auto-cleans Rust; the Dioxus prompt hits closed stdin => "no".
	run(t.path(), &["--yes-cargo"]);

	assert!(!t.path().join("rs/target").exists(), "rust project cleaned");
	assert!(t.path().join("dx/target").exists(), "dioxus project untouched");
}

#[test]
fn yes_dioxus_cleans_dioxus() {
	let t = tmp();
	make_dioxus(&t.path().join("dx"), "dx");
	make_build_dir(&t.path().join("dx/target"));

	run(t.path(), &["--yes-dioxus"]);

	assert!(!t.path().join("dx/target").exists(), "dioxus project cleaned");
}

#[test]
fn pruned_directories_are_not_searched() {
	let t = tmp();
	// A real project with a build dir, buried inside node_modules.
	let buried = t.path().join("node_modules/pkg");
	make_crate(&buried, "pkg");
	make_build_dir(&buried.join("target"));

	run(t.path(), &["--orphans", "--yes-all"]);

	assert!(
		buried.join("target").exists(),
		"node_modules must be pruned, so its build dir is never touched"
	);
}

#[test]
fn nested_independent_workspace_is_also_cleaned() {
	let t = tmp();
	let a = t.path().join("a");
	make_workspace(&a, &["m"]);
	make_crate(&a.join("m"), "m");
	make_build_dir(&a.join("target"));

	// A crate with its own `[workspace]` nested under A — cargo treats it as a
	// separate workspace (auto-excluded from A).
	let b = a.join("vendor/b");
	write(
		&b.join("Cargo.toml"),
		"[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n",
	);
	write(&b.join("src/lib.rs"), "");
	make_build_dir(&b.join("target"));

	let o = run(t.path(), &["--yes-all"]);

	assert!(o.status.success());
	assert!(!a.join("target").exists(), "outer workspace cleaned");
	assert!(!b.join("target").exists(), "nested independent workspace cleaned");
}
