# Handoff: review & polish pass on `rustsweep`

> **Addendum (2026-07-23, after stages 0–2 of `docs-and-release-plan.md`):**
> The tree is now **18 unit + 48 integration + 3 docs tests**, `dist plan`
> announces **v0.1.4**, and `cargo publish --dry-run` is clean.
>
> Two things from §6 are resolved. **CI was red on its first run** — `cargo fmt
> --check` failed on the runner because the repo had no `rustfmt.toml` and the
> tabs/120-column style was coming from a machine-global config at
> `~/Library/Application Support/rustfmt/rustfmt.toml`. The style is now checked
> in, stable-only keys, and stable and nightly rustfmt produce identical output.
> Dropping the nightly-only `short_array_element_width_threshold` is why the
> arg-id array in `cli.rs` is now vertical. **`repository`/`homepage` pointed at
> `github.com/Rottschaferanders/rustsweep`, which does not exist** — the repo is
> under `rootschafer`. Fixed in Cargo.toml and the README.
>
> New in the tree, all guarded rather than maintained by hand:
> `CHANGELOG.md` (dist finds it on its own and uses the matching section as the
> release body); `tests/docs.rs`, which renders the README usage block,
> `docs/src/cli-reference.md`, and `docs/man/rustsweep.1` from
> `cli::docs_command()` and diffs them — `UPDATE_DOCS=1 cargo test` re-blesses,
> hand-editing any of the three is a test failure; a hidden `--completions
> <shell>` flag; and an mdBook site under `docs/` deployed by
> `.github/workflows/docs.yml`. Help wrapping is pinned at 100 columns from both
> ends (`max_term_width` on the command, `term_width` in `docs_command`) so a
> wide terminal can't produce help the checked-in copies don't match.
>
> **The man page embeds the version**, so a version bump without
> `UPDATE_DOCS=1 cargo test` turns CI red. It's in the README's release steps.
>
> Still outstanding, all needing Anders: `git push`, tagging `v0.1.4`,
> **repo Settings → Pages → Source: "GitHub Actions"** (the docs deploy 404s
> until then), and the crates.io publish decision.
>
> **Addendum (2026-07-22, after the polish pass):** items 1, 2, 5, 6, and 7 of
> §5 are done — `measure()` now reports incomplete reads and the filters keep
> anything not fully measured (unit + integration tested); `freed` only counts
> removals that succeeded; `resolve_in_parallel` returns a named
> `ResolvedProject`; the "(Cargo project)" parenthetical is gone; the README
> notes apparent sizes. §5.3 was decided as "explicit `--path` beats the ignore
> list", documented in README/`discover.rs` and locked by a test. §5.4 (plan.rs
> scaling) intentionally untouched, per the "measure first" note. From §6:
> `license` in Cargo.toml now says `MIT` to match the committed `LICENSE.txt`
> (flag if dual-licensing was actually wanted), and a test CI workflow exists
> at `.github/workflows/ci.yml` (fmt/clippy/test on ubuntu+macos). Counts are
> now 18 unit + 46 integration tests.

**Written:** 2026-07-22, after the config-file work, a Dioxus-removal cleanup, the
glob ignore rewrite, the `rust-cleanup` → `rustsweep` rename, and dist setup.
**Audience:** the next AI (or human) doing a review/polish pass.
**Status of the tree right now:** 14 unit + 44 integration tests pass,
`cargo clippy --all-targets` is clean, `cargo fmt` applied, `dist plan` and a
local `dist build` both work. Nothing is known-broken. This is a *polish* brief,
not a bug hunt with a known target.

---

## 1. What the tool does

Point it at a directory; it finds every Cargo project underneath, cleans each
workspace exactly once via `cargo clean`, and separately removes build dirs
`cargo clean` cannot reach. Nothing is deleted without a `y` unless `--yes`.

The load-bearing idea: a directory is a Cargo build dir **iff** it holds a
`CACHEDIR.TAG` whose body contains "cargo". That's how a renamed `target/`, a
`--target-dir` leftover, or a dir from an old `build.target-dir` gets found, and
it's what keeps us from ever deleting an unrelated directory named `target` or
another tool's cache. **Never loosen this test.**

## 2. Architecture (`src/`, ~1150 lines)

Pipeline, in `cli::run_cli()` order:

| File | Role |
| --- | --- |
| `cli.rs` | clap types, config↔CLI merge (`resolve`), rayon pool sizing |
| `config.rs` | the optional TOML file: shape, discovery, loading |
| `ignore.rs` | the ignore list: pattern compilation + matching |
| `discover.rs` | the parallel walk → `Discovery { projects, build_dirs }` |
| `plan.rs` | `cargo metadata` per project → `Vec<Workspace>` |
| `clean.rs` | executes the plan; **every deletion funnels through `Tally::consider`** |
| `util.rs` | `canonical_or`, `expand_tilde`, `parse_size` |

`discover` → `plan` → `clean` is a straight line; each stage's output is the
next's input, and none of them call back.

### Invariants worth not breaking

1. **Deletions have exactly one code path** (`Tally::consider`). It was three
   near-identical blocks before; keep it one, and keep new target kinds as
   variants of `Target`.
2. **The default walk pays nothing for features it isn't using.** Sizing/age
   measurement only happens when `--show-size`/`--keep-*` is set
   (`measure_if_needed`). The ignore matcher is two-tier for the same reason —
   see §3. Measured on a 60,693-dir tree: 8.8s default, 13.0s with one glob
   configured. A change that puts a per-directory syscall on the default path is
   a regression even if tests stay green.
3. **`--yes` is command-line only, by design.** It is deliberately absent from
   `Config`, and `resolve()` reads it straight off the CLI with no merge. Do not
   "restore symmetry" here.
4. **Ignore lists are additive.** `--ignore` extends the config's list; built-in
   names (`.git`, `node_modules`, `.jj`) can be added to but never removed.
5. **Config precedence is CLI > config > default**, per key. Bools go through
   `merge_bool`, which asks clap `value_source` — a bare `false` can't be
   distinguished from "unset" otherwise. `cli::tests::flag_ids_are_the_field_names`
   guards the arg-id strings those lookups depend on; if it fails, precedence is
   silently broken, so treat it as load-bearing rather than noise.

### The ignore matcher (`ignore.rs`), since it's the least obvious file

`.gitignore`-style patterns from the config's `ignore` key and `--ignore`. A
pattern with no `/` and no glob metacharacter is a **bare name**, kept in a
`HashSet` and compared against each directory's file name — free, and where the
built-ins live. Anything else is compiled into a `GlobSet` matched against the
directory's **absolute path**, which costs a canonicalization per directory;
`has_globs` gates that entirely, so a name-only config never touches the
filesystem. `literal_separator(true)` is what makes `*` stop at `/` while `**`
spans. Each glob is added twice (`P` and `P/**`) so ignoring a directory also
covers its contents. `matches()` tries both the walked path and its canonical
form because only one may resemble what the user wrote (macOS `/var` →
`/private/var`; a relative `--path` yields relative paths).

## 3. Conventions

- **Tabs** for indentation. `cargo fmt` is configured and enforced; run it.
- Internal items are `pub(crate)`; only `cli` is `pub`.
- Comments explain *why*, not *what*. The existing density is the target — match
  it rather than stripping or padding.
- Tests: integration tests spawn the real binary
  (`env!("CARGO_BIN_EXE_rustsweep")`) against fixture trees in a `TempDir`;
  unit tests live inline in their module.
- **Test hermeticity:** `run`/`run_input` set `RUSTSWEEP_CONFIG` to a
  nonexistent path so a real `~/.config/rustsweep/config.toml` can't perturb
  them. Any new helper that spawns the binary must do the same.

### macOS test-harness gotcha (will cost you 10 minutes if you don't know)

A failing assertion aborts with `fatal runtime error: failed to initiate panic,
error 5` and **swallows the assertion message**, and with parallel threads it
won't even say which test failed. To find it:
`cargo test --test integration -- --test-threads=1`, then
`cargo test --test integration <name> -- --nocapture` to read the message.

## 4. Verify with

```sh
cargo test                     # 18 unit + 48 integration + 3 docs
cargo clippy --all-targets     # must be silent
cargo fmt --check              # uses the repo's rustfmt.toml, same as CI
dist plan                      # release config still coherent
mdbook build docs              # the book still builds
.claude/skills/run-rustsweep/smoke.sh
```

## 5. Polish candidates, most-worth-it first

Nothing here is known-broken; these are the soft spots I'd look at.

1. **`filters_allow`'s "never delete what we failed to inspect" guarantee isn't
   real** (`clean.rs`). Its doc comment says a filter with no measurement keeps
   the dir, and there's a `let ... else { return false }` for it — but that
   branch is now unreachable: `measure_if_needed` returns `Some` whenever any
   filter is set, and `measure()` swallows `read_dir` errors, so an unreadable
   build dir measures as `(0 bytes, UNIX_EPOCH)`. With `--keep-days` that reads
   as *ancient* → it gets deleted; with `--keep-size` as *tiny* → kept. **Decide
   the intended semantics and make code and comment agree.** If the guarantee is
   wanted, `measure` needs to report failure rather than returning zeros.
2. **`freed` counts bytes that may not have been freed.** `Tally::consider` adds
   `size` after `cargo_clean`/`remove_dir`, both of which only *print* on
   failure. A failed removal still inflates the "Freed ~X" total. Either thread
   a success bool back or say "attempted".
3. **The walk root is never tested against the ignore set.** Only subdirectories
   are filtered, so `rustsweep --path /some/ignored/dir` happily scans it. That
   may well be the right behavior (an explicit `--path` beats a config), but
   it's undocumented and untested either way — decide and write it down.
4. **`plan.rs` scaling.** `handled_roots` is a `Vec` scanned with
   `.any(|root| dir.starts_with(root))` per candidate, and `containing_project`
   is a linear scan per build dir. Fine at today's scale (~1400 projects on the
   author's machine, unmeasurable) but both are quadratic in shape. Only worth
   touching with a measurement in hand.
5. **`resolve_in_parallel`'s return type** is
   `Vec<(&PathBuf, Result<(PathBuf, PathBuf, Vec<PathBuf>), String>)>` with an
   `#[allow(clippy::type_complexity)]` on it. A small named struct would read
   better and let the `allow` go.
6. **Message wording.** "Would clean X (Cargo project)" — the parenthetical is
   vestigial now that Rust/Dioxus aren't distinguished, and could just go.
7. **`measure()` reports apparent size** (sum of file lengths), not on-disk
   blocks, so `--show-size` totals differ from `du`. Probably fine; worth a
   README sentence.

## 6. Open decisions (do not silently resolve these)

- **Licensing.** `Cargo.toml` declares `license = "MIT OR Apache-2.0"` because
  `dist init` and crates.io need it, but **there are no `LICENSE-MIT` /
  `LICENSE-APACHE` files in the repo.** Anders should confirm the choice before
  publishing; then add the files.
- **No test CI.** `.github/workflows/release.yml` is the only workflow — nothing
  runs `cargo test`/clippy/fmt on push or PR. Offered and not yet accepted.
- **`--no-<flag>` overrides** (to switch a config-enabled bool back off for one
  run) were deferred, not rejected. Only needed if someone asks.
- **crates.io publish** hasn't happened. `rustsweep` was unclaimed as of
  2026-07-22; that is not a reservation.

## 7. Things to leave alone

- **`.github/workflows/release.yml` is generated.** Edit `dist-workspace.toml`
  and re-run `dist generate`; hand edits get overwritten.
- **The three generated doc artifacts** — the README's usage block (between the
  `BEGIN GENERATED` / `END GENERATED` markers), `docs/src/cli-reference.md`, and
  `docs/man/rustsweep.1`. `UPDATE_DOCS=1 cargo test` is the only way to change
  them; `tests/docs.rs` fails otherwise.
- **`rustfmt.toml` is stable-only on purpose.** Adding a nightly-only key puts
  local formatting back out of step with CI, which is the bug it was added to
  fix.
- **Don't reintroduce project-type detection.** `dx clean` no longer exists, so
  Rust vs. Dioxus is a distinction without a difference. `ProjectType`,
  `Candidate`, and `workspace_kind` were deliberately deleted;
  `Discovery.projects` is a plain `Vec<PathBuf>` now.
- **`notes/config-file-plan.md`** is a historical record with a "superseded"
  banner — parts of it (`ignore_paths`/`ignore_names`, config `yes`) describe a
  design that no longer exists. Read README + `config.example.toml` for current
  behavior; don't implement from that plan.
