---
name: run-rustsweep
description: Build, run, and smoke-test rustsweep. Use when asked to run rustsweep, try the CLI, verify a change works in the real binary, or exercise its prompts/flags end-to-end.
---

rustsweep is a single-binary CLI that finds Cargo projects under a directory
and deletes their build dirs (interactively, unless `--yes`). **It is a
deletion tool: never point a write-mode run at a real directory. Drive it
against a fixture tree, or use `--dry-run`.** The harness for that is
`.claude/skills/run-rustsweep/smoke.sh`. All paths are relative to the repo
root.

## Prerequisites

A Rust toolchain (`cargo` on PATH) — nothing else. Developed on macOS; the
suite also runs on Linux and Windows in CI.

## Build

```bash
cargo build          # binary at target/debug/rustsweep
```

## Run (agent path)

The smoke driver builds the binary, fabricates throwaway Cargo projects and
tagged build dirs under `mktemp -d`, and drives every main flow — dry-run,
the interactive y/n prompt (via piped stdin), `--yes`, `--orphans`, the
non-cargo-cache safety rule, and a `--keep-size` filter:

```bash
.claude/skills/run-rustsweep/smoke.sh
# → per-scenario "== ..." lines, then "SMOKE OK" (exit 0). Any failure
#   prints "SMOKE FAIL: <reason>" and exits 1.
```

To poke a single flow by hand, build a fixture the same way the driver does
(a crate is just a `Cargo.toml` + empty `src/lib.rs`; a build dir is any dir
holding a `CACHEDIR.TAG` whose body contains "cargo") and run against it:

```bash
./target/debug/rustsweep --path "$FIXTURE" --dry-run --show-size
# → Scanned 4 directories: found 1 project(s) and 1 build dir(s).
#   Dry run — nothing will be deleted.
#   Would clean /…/a — build dir /…/a/target (2.0 MiB)
#   Would free ~2.0 MiB.
```

Drive the interactive prompt through a pipe — `y` cleans, `n` keeps and
lists the path under "Skipped", closed stdin (EOF) answers "no":

```bash
printf 'y\n' | ./target/debug/rustsweep --path "$FIXTURE"
```

Always set `RUSTSWEEP_CONFIG` to a nonexistent path first (the driver does
this) — see Gotchas.

## Run (human path)

`cargo run -- --path ~/Code --dry-run --show-size` for a look; drop
`--dry-run` to be prompted per project. Interactive prompts need a real
terminal-ish stdin.

## Test

```bash
cargo test                     # 18 unit + 48 integration + 3 docs, ~2s
                               # (Windows: 17 + 44 — some tests are cfg-gated)
cargo clippy --all-targets     # silent
cargo fmt --check
```

The `docs` suite (`tests/docs.rs`) diffs the checked-in CLI documentation — the
README usage block, `docs/src/cli-reference.md`, `docs/man/rustsweep.1` —
against what the clap definitions render right now. After any change to a flag,
its help text, or the version, regenerate rather than hand-editing:

```bash
UPDATE_DOCS=1 cargo test       # rewrites the three files in place
```

## Gotchas

- **Your real `~/.config/rustsweep/config.toml` silently changes runs.**
  Any manual invocation should `export RUSTSWEEP_CONFIG=/nonexistent` (the
  smoke driver and the integration tests both do). A config with `ignore`
  patterns or `show_size` will otherwise perturb your assertions.
- **A bare `target/` directory is not enough for the scan.** rustsweep only
  treats a dir as a build dir if it contains a `CACHEDIR.TAG` whose body
  mentions "cargo" (the smoke driver's `make_build_dir` writes the real
  one). An untagged `target/` is still cleaned, but only via `cargo clean`
  for a resolvable project — orphan/stray scenarios need the tag.
- **The binary shells out to real `cargo`** (`cargo metadata` per project,
  `cargo clean` per accepted prompt), so fixture crates must be valid
  manifests, and `cargo clean`'s own "Removed N files" line appears in
  stderr mid-run.
- **On macOS, a failing integration test aborts with
  `fatal runtime error: failed to initiate panic, error 5`** and swallows
  the assertion message (documented in `notes/review-handoff.md` §3). To
  find the culprit: `cargo test --test integration -- --test-threads=1`,
  then re-run the named test with `--nocapture`.
