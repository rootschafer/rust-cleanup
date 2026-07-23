# rustsweep

[![CI](https://github.com/rootschafer/rustsweep/actions/workflows/ci.yml/badge.svg)](https://github.com/rootschafer/rustsweep/actions/workflows/ci.yml)
[![Docs](https://github.com/rootschafer/rustsweep/actions/workflows/docs.yml/badge.svg)](https://rootschafer.github.io/rustsweep/)

A simple tool to save space on your computer by cleaning up the build artifacts
of Rust projects. Point it at a directory; it finds every Cargo project beneath
it, cleans each workspace once, and also removes the stray build dirs `cargo
clean` can't reach — renamed `target/`s, dirs left behind by
`--target-dir`, and (behind `--orphans`) build dirs with no project around them
at all. Already-clean projects are skipped. Nothing is deleted without a `y`
unless you pass `--yes`.

**📖 [Documentation](https://rootschafer.github.io/rustsweep/)** — installation,
a walkthrough of the first run, the config file, ignore patterns, and
[how it decides what to delete](https://rootschafer.github.io/rustsweep/how-it-decides.html).

# Install

No Rust toolchain needed — the installer fetches a prebuilt binary:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rootschafer/rustsweep/releases/latest/download/rustsweep-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rootschafer/rustsweep/releases/latest/download/rustsweep-installer.ps1 | iex"
```

Or from source:

```sh
cargo install --git https://github.com/rootschafer/rustsweep
```

Per-platform archives and checksums are on the
[releases page](https://github.com/rootschafer/rustsweep/releases); see the
[installation chapter](https://rootschafer.github.io/rustsweep/installation.html)
for the full set.

# Usage

Output of `rustsweep -h` — generated from the source, so it can't go stale.
`rustsweep --help` prints the same options with fuller descriptions:

<!-- BEGIN GENERATED: rustsweep -h -->
```
Frees disk space by cleaning the build artifacts of every Rust project under a directory, including
the stray build dirs `cargo clean` can't reach.

Usage: rustsweep [OPTIONS]

Options:
  -p, --path <PATH>        Sets the starting directory for the search [default: .]
  -L, --follow-symlinks    Follow symlinked directories while searching (off by default so the
                           search can't escape the tree; a cycle guard and --max-depth bound the
                           traversal)
  -d, --max-depth <DEPTH>  Limit how many directory levels below the search root to descend
      --ignore <PATTERN>   Never scan (or clean) anything matching this .gitignore-style pattern: a
                           bare name (`vendor`) matches at any depth, anything else is a glob
                           matched against the full path (`~/Code/*/target`). Repeatable; adds to
                           the config file's `ignore` list rather than replacing it
  -y, --yes                Clean everything found without prompting for a yes or a no. Command-line
                           only — this one can't be set in the config file
      --orphans            Also remove Cargo build dirs that aren't inside any discovered project
                           (e.g. left over from `cargo build --target-dir <dir>`)
  -n, --dry-run            Show what would be cleaned without deleting anything or prompting
  -v, --verbose            List the projects that `cargo metadata` could not read
  -s, --show-size          Show each build dir's size before you decide (and a freed-space total).
                           Measuring walks every target, so this is slower than a normal run
      --keep-days <DAYS>   Keep (don't clean) build dirs touched within the last N days; only clean
                           ones untouched for longer. Protects projects you're actively building
      --keep-size <SIZE>   Only clean build dirs at least this large; keep smaller ones. Accepts
                           units, e.g. 500MB, 1GiB (1024-based). Measuring sizes walks each target,
                           so this is slower than a normal run
  -h, --help               Print help
  -V, --version            Print version

Defaults for these options can be set in ~/.config/rustsweep/config.toml (a command-line flag always
wins). That file also holds the global ignore list; --ignore adds to it rather than replacing it.
```
<!-- END GENERATED -->

A good first run — see what's there without touching anything:

```sh
rustsweep --path ~/Code --dry-run --show-size
```

## Shell completions

`rustsweep --completions <shell>` writes a completion script to stdout for
`bash`, `zsh`, `fish`, `elvish`, or `powershell`. It's generated from the live
command definition, so it can't fall behind the flags:

```sh
rustsweep --completions zsh > ~/.zfunc/_rustsweep     # then: fpath+=~/.zfunc
rustsweep --completions bash > /etc/bash_completion.d/rustsweep
```

A man page ships in every release archive; `man ./rustsweep.1` reads it without
installing.

# Configuration

Persistent defaults can live in an **optional** config file at
`~/.config/rustsweep/config.toml` (or `$XDG_CONFIG_HOME/rustsweep/config.toml`
if that's set; `$RUSTSWEEP_CONFIG` overrides both). With no config file,
behavior is exactly as it is without one — nothing to set up.

**Precedence is CLI flag > config value > built-in default**, per setting. So a
config can turn something on for every run, and passing the flag explicitly
still wins for that run. Every key is optional and named after its flag:

```toml
# path = "~/Code"       # default search dir when --path is omitted
# max_depth = 8
orphans   = false
show_size = true
# keep_days = 14        # keep dirs touched within the last N days
# keep_size = "500MiB"  # only clean dirs at least this big

# Never scanned, so never cleaned — .gitignore-style patterns.
ignore = ["vendor", "~/Code/Rust/EMBED/ESP", "**/build-cache"]
```

**There is no `yes` key.** Cleaning without a prompt is the one irreversible
thing this tool does, so `--yes` has to be given per run rather than left armed
in a file. An unknown key is an error, so a typo can't silently do nothing.

`ignore` (and the repeatable `--ignore <PATTERN>`, which *adds to* it rather than
replacing it) takes `.gitignore`-style patterns — a bare name matches a directory
at any depth, anything else is a glob on the full path. `.git`, `node_modules`,
and `.jj` are always ignored.

Two things worth knowing before you rely on the numbers:

- `--show-size`, `--keep-days`, and `--keep-size` each measure every build dir,
  which walks its whole tree. A run without them never pays that.
- Sizes are *apparent* sizes (the sum of file lengths), so totals differ a little
  from `du`. A build dir that couldn't be fully read shows its size as a lower
  bound (`≥`) and is kept rather than judged on incomplete numbers.

Full detail — the annotated
[`config.example.toml`](config.example.toml), the pattern rules and their cost,
the filter semantics — is in the book:
[Configuration](https://rootschafer.github.io/rustsweep/configuration.html),
[Ignore patterns](https://rootschafer.github.io/rustsweep/ignore-patterns.html),
[Filters](https://rootschafer.github.io/rustsweep/filters.html).

# Documentation

The book lives in [`docs/`](docs/) and is published to
<https://rootschafer.github.io/rustsweep/> by `.github/workflows/docs.yml` on
every push to `main`.

```sh
mdbook serve docs      # live preview at http://localhost:3000
mdbook build docs      # one-shot build into docs/book/ (gitignored)
```

**The `docs` argument is not optional.** mdBook looks for `book.toml` in the
current directory, so a bare `mdbook build` from the repo root fails with
`failed to read .../src/SUMMARY.md` — it's reporting a path it inferred from
the wrong directory, not a missing file. Pass `docs`, or `cd docs` first.

`docs/src/cli-reference.md` is **generated** — see below. Everything else is
hand-written. `configuration.md` pulls in the real `config.example.toml` with
`{{#include}}` rather than copying it.

## Generated docs

The usage block above, `docs/src/cli-reference.md`, and `docs/man/rustsweep.1`
are all rendered from the clap definitions in `src/cli.rs` and checked by
`tests/docs.rs`. Editing any of them by hand is a test failure. After changing a
flag, its help text, or the version:

```sh
UPDATE_DOCS=1 cargo test      # rewrites all three; commit the result
```

# Releasing

Releases are built by [dist](https://github.com/axodotdev/cargo-dist). Its config
lives in `dist-workspace.toml`, and `.github/workflows/release.yml` is generated
from it — **edit the config, then re-run `dist generate`; never hand-edit the
workflow.** Current targets: macOS (arm64 + x86_64), Linux (arm64 + x86_64), and
Windows (x86_64), plus shell and PowerShell installers.

To cut a release:

```sh
dist plan                     # preview exactly what CI will produce
# bump `version` in Cargo.toml and add its CHANGELOG.md section
UPDATE_DOCS=1 cargo test      # the man page carries the version; refresh it
git commit -am "Release 0.1.4"
git tag v0.1.4
git push --tags               # this is what triggers the release workflow
```

`dist` reads `CHANGELOG.md` and uses the section matching the tag as the GitHub
Release body, so a release with no section gets an empty one.

The workflow builds every target, then creates a GitHub Release with the
archives, checksums, and the installer scripts attached.

Useful locally:

- `dist build` — build the artifacts for the current platform only, into
  `target/distrib/`. Good for checking an archive's contents before tagging.
- `dist init` — re-run the setup wizard after changing package metadata.

Upgrading dist: install the new version, bump `cargo-dist-version` in
`dist-workspace.toml`, then `dist generate`.
