//! Keeps the checked-in CLI documentation honest.
//!
//! Every artifact here is rendered from `rustsweep::cli::docs_command()` — the
//! same clap definition the binary parses with — and diffed against the copy in
//! the repo. `UPDATE_DOCS=1 cargo test` rewrites the copies instead of failing,
//! so the generator and the guard are one piece of code rather than two that can
//! drift apart. Nothing here is ever edited by hand.

use std::{
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
};

use rustsweep::cli::docs_command;

/// Set this to rewrite the checked-in files instead of asserting against them.
const BLESS_VAR: &str = "UPDATE_DOCS";

/// Where in the README the generated help lives. The markers are load-bearing:
/// the splice is between them, and prose either side is untouched.
const README_BEGIN: &str = "<!-- BEGIN GENERATED: rustsweep -h -->";
const README_END: &str = "<!-- END GENERATED -->";

#[test]
fn readme_usage_block_matches_help() {
	let path = repo_path("README.md");
	let readme = read(&path);
	let block = format!("```\n{}```\n", help_text());

	check(&path, &splice(&readme, &block), "README usage block");
}

#[test]
fn cli_reference_matches_help() {
	let path = repo_path("docs/src/cli-reference.md");
	let mut rendered = String::from(
		"<!-- Generated from the clap definitions by tests/docs.rs.\n     Do not edit; run `UPDATE_DOCS=1 cargo test`. -->\n\n",
	);
	rendered.push_str(&clap_markdown::help_markdown_command(&docs_command()));

	check(&path, &rendered, "CLI reference");
}

#[test]
fn man_page_matches_help() {
	let path = repo_path("docs/man/rustsweep.1");
	let mut rendered = Vec::new();
	clap_mangen::Man::new(docs_command())
		.render(&mut rendered)
		.expect("rendering to a Vec cannot fail");
	let rendered = String::from_utf8(rendered).expect("mangen emits UTF-8");

	check(&path, &rendered, "man page");
}

/// The short (`-h`) help, rendered at the fixed documentation width. The README
/// is the front door, so it gets the one-line-per-option form; the long help's
/// full paragraphs live in the book's CLI reference instead.
fn help_text() -> String {
	docs_command().render_help().to_string()
}

/// Replaces the marked region of the README with `block`.
fn splice(readme: &str, block: &str) -> String {
	let (before, rest) = readme
		.split_once(README_BEGIN)
		.unwrap_or_else(|| panic!("README.md is missing the `{README_BEGIN}` marker"));
	let (_, after) = rest
		.split_once(README_END)
		.unwrap_or_else(|| panic!("README.md is missing the `{README_END}` marker"));

	format!("{before}{README_BEGIN}\n{block}{README_END}{after}")
}

/// Asserts the file matches what we just rendered — or, when blessing, makes it
/// match. `what` names the artifact in the failure message.
fn check(path: &Path, rendered: &str, what: &str) {
	if blessing() {
		if let Some(dir) = path.parent() {
			fs::create_dir_all(dir).expect("creating the docs directory");
		}
		fs::write(path, rendered).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
		return;
	}

	let current = read(path);
	if current == rendered {
		return;
	}

	// Written straight to the fd rather than via `eprintln!`, which the test
	// harness captures — and on macOS a failing assertion aborts the process
	// hard enough to lose the captured output along with the panic message
	// (see notes/review-handoff.md §3). This line survives that.
	let _ = io::stderr().write_all(
		format!(
			"\nthe {what} in {} no longer matches the clap definitions.\n\
			 Run `{BLESS_VAR}=1 cargo test` to regenerate it, then commit the result.\n\n",
			path.display(),
		)
		.as_bytes(),
	);
	panic!("{what} is out of date");
}

fn blessing() -> bool {
	std::env::var_os(BLESS_VAR).is_some_and(|v| !v.is_empty())
}

/// Reads a checked-in artifact, normalizing line endings.
///
/// These guards are about *content* — whether the file still says what the clap
/// definitions render — not about which line ending the working tree uses.
/// `.gitattributes` pins that to LF, but a checkout predating it (or one whose
/// git config overrides it) would otherwise fail all three guards with a
/// misleading "out of date", and blessing would rewrite every line.
fn read(path: &Path) -> String {
	let text = fs::read_to_string(path).unwrap_or_else(|e| {
		panic!(
			"reading {}: {e}\nIf the file is missing, run `{BLESS_VAR}=1 cargo test` to generate it.",
			path.display()
		)
	});
	text.replace("\r\n", "\n")
}

/// Resolves against the manifest directory so these tests don't care about cwd.
fn repo_path(relative: &str) -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}
