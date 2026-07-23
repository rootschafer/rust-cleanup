<!-- Generated from the clap definitions by tests/docs.rs.
     Do not edit; run `UPDATE_DOCS=1 cargo test`. -->

# Command-Line Help for `rustsweep`

This document contains the help content for the `rustsweep` command-line program.

**Command Overview:**

* [`rustsweep`↴](#rustsweep)

## `rustsweep`

Frees disk space by cleaning the build artifacts of every Rust project under a directory, including the stray build dirs `cargo clean` can't reach.

**Usage:** `rustsweep [OPTIONS]`

Defaults for these options can be set in ~/.config/rustsweep/config.toml (a command-line flag always wins). That file also holds the global ignore list; --ignore adds to it rather than replacing it.

###### **Options:**

* `-p`, `--path <PATH>` — Sets the starting directory for the search

  Default value: `.`
* `-L`, `--follow-symlinks` — Follow symlinked directories while searching (off by default so the search can't escape the tree; a cycle guard and --max-depth bound the traversal)
* `-d`, `--max-depth <DEPTH>` — Limit how many directory levels below the search root to descend
* `--ignore <PATTERN>` — Never scan (or clean) anything matching this .gitignore-style pattern: a bare name (`vendor`) matches at any depth, anything else is a glob matched against the full path (`~/Code/*/target`). Repeatable; adds to the config file's `ignore` list rather than replacing it
* `-y`, `--yes` — Clean everything found without prompting for a yes or a no. Command-line only — this one can't be set in the config file
* `--orphans` — Also remove Cargo build dirs that aren't inside any discovered project (e.g. left over from `cargo build --target-dir <dir>`)
* `-n`, `--dry-run` — Show what would be cleaned without deleting anything or prompting
* `-v`, `--verbose` — List the projects that `cargo metadata` could not read
* `-s`, `--show-size` — Show each build dir's size before you decide (and a freed-space total). Measuring walks every target, so this is slower than a normal run
* `--keep-days <DAYS>` — Keep (don't clean) build dirs touched within the last N days; only clean ones untouched for longer. Protects projects you're actively building
* `--keep-size <SIZE>` — Only clean build dirs at least this large; keep smaller ones. Accepts units, e.g. 500MB, 1GiB (1024-based). Measuring sizes walks each target, so this is slower than a normal run



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
