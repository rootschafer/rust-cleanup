# rustsweep

A simple tool to save space on your computer by cleaning up the build artifacts
of Rust projects. Point it at a directory; it finds every Cargo project beneath
it, cleans each workspace once, and also removes the stray build dirs `cargo
clean` can't reach — renamed `target/`s, dirs left behind by
`--target-dir`, and (behind `--orphans`) build dirs with no project around them
at all. Already-clean projects are skipped. Nothing is deleted without a `y`
unless you pass `--yes`.

# Install

From source:

```sh
git clone https://github.com/rootschafer/rustsweep
cd rustsweep
cargo install --path .
```

Once a version is tagged, the release workflow also publishes prebuilt binaries
and a one-line installer (no Rust toolchain needed) — see [Releasing](#releasing).

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

See [`config.example.toml`](config.example.toml) for the annotated full set.

**There is no `yes` key.** Cleaning without a prompt is the one irreversible
thing this tool does, so `--yes` has to be given per run rather than left armed
in a file.

## Ignore patterns

`ignore` (and the repeatable `--ignore <PATTERN>` flag, which *adds to* it rather
than replacing it) takes `.gitignore`-style patterns:

| Pattern | Matches |
| --- | --- |
| `vendor` | a directory named `vendor`, at any depth |
| `**/build-cache` | the same thing, written as an explicit glob |
| `~/Code/*/scratch` | a glob on the full path — `*` stops at a `/`, `**` spans them |
| `/opt/toolchains` | that directory and everything under it |

A leading `~` is expanded, and `./x` resolves against the current directory.
Matching a directory prunes the whole subtree, so nothing inside it is ever
scanned or cleaned. `.git`, `node_modules`, and `.jj` are always ignored;
patterns add to that floor and can't remove from it.

Bare names cost nothing — they're compared against directory names during the
walk, exactly like the built-ins. A pattern containing `/` or a glob character is
matched against each directory's full path, which needs a path resolution per
directory (on a 60k-directory tree that's roughly a 45% slower scan). Prefer a
bare name when it says what you mean.

The search root itself is exempt: pointing `--path` directly at an ignored
directory scans it anyway — an explicit target on the command line beats the
ignore list. The patterns still apply to the directories beneath it.

## Notes

- An unknown key is an error, so a typo can't silently do nothing. A broken
  config prints a warning naming the file and the problem, then the run
  continues with the built-in defaults.
- `--show-size`, `--keep-days`, and `--keep-size` each have to measure every
  build dir, which walks its whole tree. A run without them never pays that.
- Sizes are *apparent* sizes (the sum of file lengths), so totals can differ a
  little from `du`, which counts allocated disk blocks. If part of a build dir
  can't be read, its size is shown as a lower bound (`≥`), and `--keep-days` /
  `--keep-size` keep it rather than judging it on incomplete numbers.

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
