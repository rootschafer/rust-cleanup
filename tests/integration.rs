//! End-to-end tests: build a tree of dummy Cargo projects in a temp dir, run the
//! real `rustsweep` binary against it, and assert on the resulting filesystem
//! state and output. Fixtures use fabricated build directories (a cargo-authored
//! `CACHEDIR.TAG`) so we don't need to actually compile anything — `cargo clean`
//! still removes a project's real target dir, and the scan removes strays.

use std::{
	fs,
	io::Write,
	path::Path,
	process::{Command, Output, Stdio},
};

use tempfile::TempDir;

/// Body cargo writes into a build dir's `CACHEDIR.TAG` (contains "cargo").
const CARGO_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
	# This file is a cache directory tag created by cargo.\n";
/// A generic cache tag from some other tool — must never be treated as cargo's.
const OTHER_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
	# created by some other tool\n";

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_rustsweep")
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

/// Points the binary at a config file that doesn't exist, so a real
/// `~/.config/rustsweep/config.toml` on the machine running the tests can
/// never perturb them.
fn no_config(root: &Path) -> std::path::PathBuf {
	root.join("no-such-config.toml")
}

fn run(root: &Path, args: &[&str]) -> Output {
	Command::new(bin())
		.arg("--path")
		.arg(root)
		.args(args)
		.env("RUSTSWEEP_CONFIG", no_config(root))
		.stdin(Stdio::null()) // closed stdin => any prompt answers "no"
		.output()
		.unwrap()
}

/// Runs against `config` (written next to the fixture) instead of the real one.
fn run_with_config(root: &Path, config: &str, args: &[&str]) -> Output {
	let cfg = root.join("rc-config.toml");
	fs::write(&cfg, config).unwrap();
	Command::new(bin())
		.arg("--path")
		.arg(root)
		.args(args)
		.env("RUSTSWEEP_CONFIG", &cfg)
		.stdin(Stdio::null())
		.output()
		.unwrap()
}

fn run_input(root: &Path, args: &[&str], input: &str) -> Output {
	let mut child = Command::new(bin())
		.arg("--path")
		.arg(root)
		.args(args)
		.env("RUSTSWEEP_CONFIG", no_config(root))
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

fn stderr(o: &Output) -> String {
	String::from_utf8_lossy(&o.stderr).into_owned()
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
	fs::create_dir_all(link.parent().unwrap()).unwrap();
	std::os::unix::fs::symlink(target, link).unwrap();
}

// ---------------------------------------------------------------------------

#[test]
fn already_clean_project_is_left_alone() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");

	let o = run(t.path(), &["--yes"]);

	assert!(o.status.success());
	assert!(stdout(&o).contains("were already clean"));
}

#[test]
fn standalone_project_target_is_cleaned() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	run(t.path(), &["--yes"]);

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

	let o = run(t.path(), &["--yes"]);

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

	run(t.path(), &["--yes"]);

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

	let o = run(t.path(), &["--yes", "-v"]);
	let out = stdout(&o);

	assert!(!ws.join("detached/target").exists(), "detached build dir removed by scan");
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

	run(t.path(), &["--yes"]);

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

	run(t.path(), &["--yes"]);

	assert!(!p.join("old-target").exists(), "renamed build dir removed");
}

#[test]
fn orphan_build_dir_needs_the_orphans_flag() {
	let t = tmp();
	make_build_dir(&t.path().join("loose")); // no project anywhere near it

	let o = run(t.path(), &[]);
	assert!(t.path().join("loose").exists(), "orphan kept without --orphans");
	assert!(stdout(&o).contains("orphaned"), "hint about --orphans shown");

	run(t.path(), &["--orphans", "--yes"]);
	assert!(!t.path().join("loose").exists(), "orphan removed with --orphans");
}

#[test]
fn non_cargo_cache_dir_is_never_removed() {
	let t = tmp();
	make_other_cache(&t.path().join("cache"));

	run(t.path(), &["--orphans", "--yes"]);

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

	let o = run(t.path(), &["--yes", "-v"]);
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

	let o = run(t.path(), &["--dry-run", "--orphans", "--yes"]);
	let out = stdout(&o);

	assert!(t.path().join("a/target").exists(), "dry run kept target");
	assert!(t.path().join("loose").exists(), "dry run kept orphan");
	assert!(out.contains("Would clean"));
	assert!(out.contains("Would remove orphaned"));
}

#[test]
fn pruned_directories_are_not_searched() {
	let t = tmp();
	// A real project with a build dir, buried inside node_modules.
	let buried = t.path().join("node_modules/pkg");
	make_crate(&buried, "pkg");
	make_build_dir(&buried.join("target"));

	run(t.path(), &["--orphans", "--yes"]);

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

	let o = run(t.path(), &["--yes"]);

	assert!(o.status.success());
	assert!(!a.join("target").exists(), "outer workspace cleaned");
	assert!(!b.join("target").exists(), "nested independent workspace cleaned");
}

// --- symlinks & depth -----------------------------------------------------

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_by_default() {
	let t = tmp();
	make_crate(&t.path().join("ext/proj"), "proj");
	make_build_dir(&t.path().join("ext/proj/target"));
	fs::create_dir_all(t.path().join("scan")).unwrap();
	symlink(&t.path().join("ext"), &t.path().join("scan/link"));

	run(&t.path().join("scan"), &["--yes"]);

	assert!(
		t.path().join("ext/proj/target").exists(),
		"a project reachable only via a symlink must not be touched by default"
	);
}

#[cfg(unix)]
#[test]
fn follow_symlinks_reaches_linked_projects() {
	let t = tmp();
	make_crate(&t.path().join("ext/proj"), "proj");
	make_build_dir(&t.path().join("ext/proj/target"));
	fs::create_dir_all(t.path().join("scan")).unwrap();
	symlink(&t.path().join("ext"), &t.path().join("scan/link"));

	run(&t.path().join("scan"), &["--follow-symlinks", "--yes"]);

	assert!(
		!t.path().join("ext/proj/target").exists(),
		"--follow-symlinks should reach the linked project and clean it"
	);
}

#[cfg(unix)]
#[test]
fn symlink_cycle_does_not_hang() {
	let t = tmp();
	let proj = t.path().join("proj");
	make_crate(&proj, "proj");
	make_build_dir(&proj.join("target"));
	symlink(&proj, &proj.join("loop")); // self-referential link

	// If the cycle guard were missing this would spin forever (test timeout).
	let o = run(t.path(), &["--follow-symlinks", "--yes"]);

	assert!(o.status.success());
	assert!(!proj.join("target").exists(), "project still cleaned despite the cycle");
}

#[test]
fn max_depth_bounds_the_search() {
	let t = tmp();
	// t(0) / deep(1) / proj(2)
	let proj = t.path().join("deep/proj");
	make_crate(&proj, "proj");
	make_build_dir(&proj.join("target"));

	run(t.path(), &["--max-depth", "1", "--yes"]);
	assert!(proj.join("target").exists(), "depth 1 shouldn't reach proj at depth 2");

	run(t.path(), &["--max-depth", "2", "--yes"]);
	assert!(!proj.join("target").exists(), "depth 2 reaches and cleans proj");
}

#[test]
fn max_depth_zero_scans_only_the_root() {
	let t = tmp();
	make_crate(&t.path().join("proj"), "proj");
	make_build_dir(&t.path().join("proj/target"));

	run(t.path(), &["--max-depth", "0", "--yes"]);

	assert!(t.path().join("proj/target").exists(), "--max-depth 0 should not descend at all");
}

// --- weird workspace & build-dir setups -----------------------------------

#[test]
fn glob_workspace_members_are_cleaned_once() {
	let t = tmp();
	let ws = t.path().join("ws");
	make_workspace(&ws, &["crates/*"]); // glob membership
	make_crate(&ws.join("crates/x"), "x");
	make_crate(&ws.join("crates/y"), "y");
	make_build_dir(&ws.join("target"));

	let o = run(t.path(), &["--yes"]);

	assert!(o.status.success());
	assert!(!ws.join("target").exists(), "globbed workspace cleaned at the root");
}

#[test]
fn excluded_member_build_dir_is_still_removed_quietly() {
	let t = tmp();
	let ws = t.path().join("ws");
	write(
		&ws.join("Cargo.toml"),
		"[workspace]\nresolver = \"2\"\nmembers = [\"a\"]\nexclude = [\"b\"]\n",
	);
	make_crate(&ws.join("a"), "a");
	make_crate(&ws.join("b"), "b"); // excluded from the workspace
	make_build_dir(&ws.join("b/target"));

	let o = run(t.path(), &["--yes", "-v"]);

	assert!(!ws.join("b/target").exists(), "excluded crate's build dir removed by scan");
	assert!(
		!stdout(&o).contains("couldn't be read"),
		"an excluded crate isn't a metadata failure"
	);
}

#[test]
fn untagged_target_is_still_cleaned_by_cargo() {
	let t = tmp();
	let a = t.path().join("a");
	make_crate(&a, "a");
	// A `target/` with no CACHEDIR.TAG: the scan won't flag it, but it IS the
	// resolved build dir, so pass-1 `cargo clean` must still remove it.
	write(&a.join("target/junk"), "x");

	run(t.path(), &["--yes"]);

	assert!(!a.join("target").exists(), "cargo clean removes the resolved target");
}

#[test]
fn projects_sharing_one_build_dir_are_deduped() {
	let t = tmp();
	let shared = t.path().join("shared-target");
	for name in ["p1", "p2"] {
		let p = t.path().join(name);
		make_crate(&p, name);
		write(
			&p.join(".cargo/config.toml"),
			&format!("[build]\ntarget-dir = \"{}\"\n", shared.display()),
		);
	}
	make_build_dir(&shared);

	let o = run(t.path(), &["--yes"]);

	assert!(o.status.success());
	assert!(!shared.exists(), "the shared build dir is cleaned exactly once");
}

#[test]
fn project_at_the_search_root_is_cleaned() {
	let t = tmp();
	let proj = t.path().join("proj");
	make_crate(&proj, "proj");
	make_build_dir(&proj.join("target"));

	run(&proj, &["--yes"]); // point --path straight at the crate

	assert!(!proj.join("target").exists());
}

#[test]
fn deeply_nested_workspace_members_are_covered() {
	let t = tmp();
	let ws = t.path().join("ws");
	make_workspace(&ws, &["a/b/c"]);
	make_crate(&ws.join("a/b/c"), "c");
	make_build_dir(&ws.join("target"));

	run(t.path(), &["--yes"]);

	assert!(!ws.join("target").exists());
}

#[test]
fn a_crate_inside_a_build_dir_is_ignored() {
	let t = tmp();
	let bd = t.path().join("bd");
	make_build_dir(&bd); // a cargo build dir
	// A stray crate accidentally sitting inside it, with its own build dir.
	make_crate(&bd.join("nested"), "nested");
	make_build_dir(&bd.join("nested/target"));

	run(t.path(), &["--yes"]); // no --orphans

	assert!(
		bd.join("nested/target").exists(),
		"we must not descend into a build dir, so anything inside it is untouched"
	);
}

// --- --keep-days / --keep-size filters (correct polarity) -----------------

use std::time::{Duration, SystemTime};

/// Set the mtime of every file under `dir` to `days` days ago.
fn age_build_dir(dir: &Path, days: u64) {
	let when = filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(days * 86_400));
	fn recurse(dir: &Path, when: filetime::FileTime) {
		for entry in fs::read_dir(dir).unwrap().flatten() {
			let p = entry.path();
			if entry.file_type().unwrap().is_dir() {
				recurse(&p, when);
			} else {
				filetime::set_file_mtime(&p, when).unwrap();
			}
		}
	}
	recurse(dir, when);
}

#[test]
fn keep_days_protects_recent_projects_and_cleans_stale_ones() {
	let t = tmp();
	make_crate(&t.path().join("fresh"), "fresh");
	make_build_dir(&t.path().join("fresh/target")); // mtime ≈ now
	make_crate(&t.path().join("stale"), "stale");
	make_build_dir(&t.path().join("stale/target"));
	age_build_dir(&t.path().join("stale/target"), 40); // untouched for 40 days

	run(t.path(), &["--keep-days", "30", "--yes"]);

	assert!(
		t.path().join("fresh/target").exists(),
		"recently-built project must be PROTECTED (not cleaned)"
	);
	assert!(
		!t.path().join("stale/target").exists(),
		"a project untouched for >30 days should be cleaned"
	);
}

#[test]
fn keep_size_cleans_large_targets_and_keeps_small_ones() {
	let t = tmp();
	// Big: >1 MiB of artifacts.
	make_crate(&t.path().join("big"), "big");
	make_build_dir(&t.path().join("big/target"));
	write(&t.path().join("big/target/debug/blob"), &"x".repeat(2 * 1024 * 1024));
	// Small: a few bytes.
	make_crate(&t.path().join("small"), "small");
	make_build_dir(&t.path().join("small/target"));

	run(t.path(), &["--keep-size", "1MiB", "--yes"]);

	assert!(
		!t.path().join("big/target").exists(),
		"a target at/above the size threshold should be cleaned"
	);
	assert!(
		t.path().join("small/target").exists(),
		"a target below the size threshold must be KEPT"
	);
}

#[cfg(unix)]
#[test]
fn a_filter_never_deletes_a_build_dir_it_could_not_fully_measure() {
	use std::os::unix::fs::PermissionsExt;

	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	let target = t.path().join("a/target");
	make_build_dir(&target);
	age_build_dir(&target, 40); // reads as long-stale where measurable...
	// ...but part of it can't be read at all, so its true age/size is unknown.
	let debug = target.join("debug");
	fs::set_permissions(&debug, fs::Permissions::from_mode(0o000)).unwrap();

	let o = run(t.path(), &["--keep-days", "30", "--yes"]);

	// Restore before asserting so the TempDir can clean up either way.
	fs::set_permissions(&debug, fs::Permissions::from_mode(0o755)).unwrap();

	assert!(o.status.success());
	assert!(
		target.exists(),
		"an unmeasurable build dir must be kept, not treated as ancient/empty"
	);
}

#[test]
fn filters_do_not_affect_a_normal_run() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	run(t.path(), &["--yes"]); // no filters → unchanged behavior

	assert!(!t.path().join("a/target").exists());
}

#[test]
fn show_size_reports_per_dir_size_and_a_freed_total() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));
	write(&t.path().join("a/target/debug/blob"), &"x".repeat(2 * 1024 * 1024)); // 2 MiB

	let o = run(t.path(), &["--show-size", "--dry-run"]);
	let out = stdout(&o);

	assert!(out.contains("MiB"), "per-dir size should be shown:\n{out}");
	assert!(out.contains("Would free"), "a freed total should be shown:\n{out}");
	assert!(t.path().join("a/target").exists(), "dry run deletes nothing");
}

// --- config file ----------------------------------------------------------

#[test]
fn config_can_enable_a_flag_the_cli_omits() {
	let t = tmp();
	make_build_dir(&t.path().join("loose")); // an orphan: needs --orphans

	let o = run_with_config(t.path(), "orphans = true\n", &["--yes"]);

	assert!(
		!t.path().join("loose").exists(),
		"config-set orphans should let --yes remove the orphan:\n{}",
		stdout(&o)
	);
}

#[test]
fn config_show_size_applies_without_the_flag() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));
	write(&t.path().join("a/target/debug/blob"), &"x".repeat(2 * 1024 * 1024));

	let o = run_with_config(t.path(), "show_size = true\n", &["--dry-run"]);
	let out = stdout(&o);

	assert!(out.contains("MiB"), "config show_size shows per-dir sizes:\n{out}");
	assert!(out.contains("Would free"), "and the freed total:\n{out}");
}

#[test]
fn cli_option_overrides_the_config_value() {
	let t = tmp();
	// t(0) / deep(1) / proj(2) — reachable only at depth >= 2.
	let proj = t.path().join("deep/proj");
	make_crate(&proj, "proj");
	make_build_dir(&proj.join("target"));

	let cfg = "max_depth = 1\n";

	run_with_config(t.path(), cfg, &["--yes"]);
	assert!(proj.join("target").exists(), "config's max_depth = 1 applies");

	run_with_config(t.path(), cfg, &["--max-depth", "5", "--yes"]);
	assert!(!proj.join("target").exists(), "the CLI's --max-depth wins");
}

#[test]
fn config_ignore_protects_a_subtree() {
	let t = tmp();
	let skipped = t.path().join("skip/proj");
	make_crate(&skipped, "skipped");
	make_build_dir(&skipped.join("target"));
	let normal = t.path().join("normal");
	make_crate(&normal, "normal");
	make_build_dir(&normal.join("target"));

	let cfg = format!("ignore = [\"{}\"]\n", t.path().join("skip").display());
	run_with_config(t.path(), &cfg, &["--yes"]);

	assert!(skipped.join("target").exists(), "an ignored tree is never scanned");
	assert!(!normal.join("target").exists(), "its siblings are still cleaned");
}

#[test]
fn a_bare_name_pattern_prunes_by_directory_name() {
	let t = tmp();
	let buried = t.path().join("vendor/proj");
	make_crate(&buried, "buried");
	make_build_dir(&buried.join("target"));

	run_with_config(t.path(), "ignore = [\"vendor\"]\n", &["--yes"]);

	assert!(
		buried.join("target").exists(),
		"a dir named `vendor` should be pruned anywhere in the tree"
	);
}

#[test]
fn ignore_flag_protects_a_subtree_without_a_config() {
	let t = tmp();
	let skipped = t.path().join("skip/proj");
	make_crate(&skipped, "skipped");
	make_build_dir(&skipped.join("target"));
	let normal = t.path().join("normal");
	make_crate(&normal, "normal");
	make_build_dir(&normal.join("target"));

	let skip_arg = t.path().join("skip");
	run(t.path(), &["--yes", "--ignore", skip_arg.to_str().unwrap()]);

	assert!(skipped.join("target").exists(), "--ignore protects the subtree");
	assert!(!normal.join("target").exists(), "and nothing else");
}

#[test]
fn cli_ignore_adds_to_the_configs_ignore_list() {
	let t = tmp();
	for name in ["from-config", "from-cli", "normal"] {
		let p = t.path().join(name).join("proj");
		make_crate(&p, "p");
		make_build_dir(&p.join("target"));
	}

	let cfg = format!("ignore = [\"{}\"]\n", t.path().join("from-config").display());
	let cli_ignore = t.path().join("from-cli");
	run_with_config(t.path(), &cfg, &["--yes", "--ignore", cli_ignore.to_str().unwrap()]);

	assert!(
		t.path().join("from-config/proj/target").exists(),
		"--ignore must not replace the config's list"
	);
	assert!(t.path().join("from-cli/proj/target").exists(), "--ignore applies too");
	assert!(
		!t.path().join("normal/proj/target").exists(),
		"an un-ignored project is still cleaned"
	);
}

#[test]
fn the_search_root_itself_is_exempt_from_the_ignore_list() {
	let t = tmp();
	let vendor = t.path().join("vendor");
	let proj = vendor.join("proj");
	make_crate(&proj, "proj");
	make_build_dir(&proj.join("target"));

	// Scanning from above, `vendor` is pruned like any other match...
	run_with_config(t.path(), "ignore = [\"vendor\"]\n", &["--yes"]);
	assert!(proj.join("target").exists(), "an ignored subtree is untouched");

	// ...but pointing --path straight at it is an explicit request that wins.
	run_with_config(&vendor, "ignore = [\"vendor\"]\n", &["--yes"]);
	assert!(
		!proj.join("target").exists(),
		"an explicit --path at an ignored dir must still be scanned"
	);
}

#[test]
fn malformed_config_warns_and_falls_back_to_defaults() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	let o = run_with_config(t.path(), "orphan = true\nthis is not toml\n", &["--yes"]);

	assert!(
		stderr(&o).contains("invalid config"),
		"a broken config should warn on stderr:\n{}",
		stderr(&o)
	);
	assert!(!t.path().join("a/target").exists(), "the run still proceeds normally");
}

#[test]
fn a_missing_config_behaves_like_no_config() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	let o = Command::new(bin())
		.arg("--path")
		.arg(t.path())
		.arg("--yes")
		.env("RUSTSWEEP_CONFIG", t.path().join("nope.toml"))
		.stdin(Stdio::null())
		.output()
		.unwrap();

	assert!(o.status.success());
	assert!(
		!stderr(&o).contains("Warning"),
		"a missing config is the normal case — no warning:\n{}",
		stderr(&o)
	);
	assert!(!t.path().join("a/target").exists());
}

#[test]
fn config_keep_size_filters_like_the_flag() {
	let t = tmp();
	make_crate(&t.path().join("big"), "big");
	make_build_dir(&t.path().join("big/target"));
	write(&t.path().join("big/target/debug/blob"), &"x".repeat(2 * 1024 * 1024));
	make_crate(&t.path().join("small"), "small");
	make_build_dir(&t.path().join("small/target"));

	run_with_config(t.path(), "keep_size = \"1MiB\"\n", &["--yes"]);

	assert!(!t.path().join("big/target").exists(), "large target cleaned");
	assert!(t.path().join("small/target").exists(), "small target kept");
}

#[test]
fn ignore_accepts_glob_patterns() {
	let t = tmp();
	// Two projects one level down; only the one under a `*/generated` path is skipped.
	let skipped = t.path().join("a/generated");
	make_crate(&skipped, "skipped");
	make_build_dir(&skipped.join("target"));
	let normal = t.path().join("b/kept");
	make_crate(&normal, "kept");
	make_build_dir(&normal.join("target"));

	let cfg = format!("ignore = [\"{}/*/generated\"]\n", t.path().display());
	run_with_config(t.path(), &cfg, &["--yes"]);

	assert!(skipped.join("target").exists(), "the glob should prune a/generated");
	assert!(!normal.join("target").exists(), "b/kept doesn't match the glob");
}

#[test]
fn a_relative_glob_matches_at_any_depth() {
	let t = tmp();
	let skipped = t.path().join("deep/nested/build-cache/proj");
	make_crate(&skipped, "skipped");
	make_build_dir(&skipped.join("target"));

	run_with_config(t.path(), "ignore = [\"**/build-cache\"]\n", &["--yes"]);

	assert!(skipped.join("target").exists(), "**/build-cache matches wherever it sits");
}

#[test]
fn yes_cannot_be_armed_from_the_config() {
	let t = tmp();
	make_crate(&t.path().join("a"), "a");
	make_build_dir(&t.path().join("a/target"));

	// `yes` is not a config key, so this is a typo as far as the loader is concerned:
	// it warns, falls back to defaults, and the (closed) prompt answers "no".
	let o = run_with_config(t.path(), "yes = true\n", &[]);

	assert!(stderr(&o).contains("invalid config"), "unknown key warns:\n{}", stderr(&o));
	assert!(
		t.path().join("a/target").exists(),
		"a config must never be able to delete without a prompt"
	);
}
