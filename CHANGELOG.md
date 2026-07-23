# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Versions before 0.1.4 were never tagged or published — they are reconstructed
from git history and listed for continuity, dated by their last commit.

## [Unreleased]

## [0.1.4] - 2026-07-23

The first release with prebuilt binaries and an installer.

### Added

- Documentation site built with [mdBook](https://rust-lang.github.io/mdBook/),
  published to GitHub Pages from `docs/`.
- A generated CLI reference, `docs/src/cli-reference.md`, and a man page,
  `docs/man/rustsweep.1`, both derived from the clap definitions and checked
  against them by tests. The man page ships in the release archives.
- Hidden `--completions <shell>` flag, printing a shell completion script for
  bash, zsh, fish, elvish, or PowerShell to stdout.
- The test suite now also runs on Windows in CI, alongside Linux and macOS.
- This changelog.

### Fixed

- Ignore patterns written as native Windows paths were broken twice over: a
  path like `C:\Users\me\vendor` contains no `/`, so it was read as a bare
  directory name and never matched — and globset treats `\` in a pattern as an
  escape character. Separators are now normalized on Windows.

- `repository` and `homepage` in `Cargo.toml` pointed at a GitHub URL that does
  not resolve; the README's clone command had the same typo.
- `cargo fmt --check` disagreed between a contributor's machine and CI, because
  the repo shipped no `rustfmt.toml`. The style is now checked in.

## [0.1.3] - 2026-07-23

### Changed

- Renamed from `rust-cleanup` to `rustsweep`; the binary and the config
  directory (`~/.config/rustsweep/`) changed with it.
- Ignore patterns are now `.gitignore`-style: a bare name matches a directory at
  any depth, anything else is a glob matched against the full path. Bare names
  are matched during the walk and cost nothing, so a name-only ignore list never
  touches the filesystem.
- Release automation via [dist](https://github.com/axodotdev/cargo-dist):
  prebuilt binaries for macOS, Linux, and Windows, plus shell and PowerShell
  installers.
- Project-type detection was removed. `dx clean` no longer exists, so Dioxus and
  plain Cargo projects are cleaned identically.
- CI runs `cargo fmt --check`, `cargo clippy`, and `cargo test` on Linux and
  macOS.

### Fixed

- `--keep-days` and `--keep-size` no longer judge a build dir on numbers they
  could not fully collect: a directory that could not be read all the way
  through is kept, and its size is reported as a lower bound (`≥`).
- The "Freed ~X" total only counts removals that actually succeeded.
- The search root is exempt from the ignore list — pointing `--path` at an
  ignored directory scans it — which is now documented and tested.

## [0.1.2] - 2026-07-22

### Added

- Optional config file at `~/.config/rustsweep/config.toml`
  (`$XDG_CONFIG_HOME` and `$RUSTSWEEP_CONFIG` are honored). Every key is
  optional and named after its flag; precedence is CLI flag > config value >
  built-in default. There is deliberately no `yes` key.
- `--show-size` to report each build dir's size and a freed-space total.
- `--keep-days <DAYS>` to keep build dirs touched recently, and
  `--keep-size <SIZE>` to clean only dirs at least a given size (`500MB`,
  `1GiB`).
- `--follow-symlinks` (off by default, so the search cannot escape the tree) and
  `--max-depth <DEPTH>`.

## [0.1.1] - 2026-07-21

### Changed

- Rewrote the filesystem traversal, roughly a 20x speedup. Sizing and age
  measurement happen only when a flag asks for them, so a default run pays
  nothing for them.
- Split the single binary into `discover` → `plan` → `clean` stages behind a
  library, with an integration suite driving the real binary against fixture
  trees.

## [0.1.0] - 2026-07-21

### Added

- Initial tool: find every Cargo project under a directory, clean each workspace
  once with `cargo clean`, and separately remove build dirs `cargo clean` cannot
  reach. Nothing is deleted without a `y` unless `--yes` is passed.
- Build dirs are identified by a `CACHEDIR.TAG` whose body mentions cargo, which
  is what finds a renamed `target/` or a `--target-dir` leftover without ever
  matching an unrelated directory named `target`.
- `cargo metadata` per project, run in parallel, to resolve the real target
  directory; projects it cannot read are reported at the end instead of
  aborting the run.
- `--orphans` to also remove build dirs with no project around them.

[Unreleased]: https://github.com/rootschafer/rustsweep/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/rootschafer/rustsweep/releases/tag/v0.1.4
[0.1.3]: https://github.com/rootschafer/rustsweep/compare/910232c...ffcaa0c
[0.1.2]: https://github.com/rootschafer/rustsweep/compare/43105bc...910232c
[0.1.1]: https://github.com/rootschafer/rustsweep/compare/18538db...43105bc
[0.1.0]: https://github.com/rootschafer/rustsweep/commits/18538db
