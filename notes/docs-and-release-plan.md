# Plan: shipping rustsweep to real users + docs site

**Written:** 2026-07-23, after the polish pass; revised same day after Anders
chose **mdBook over oranda** for the docs site (oranda looked unmaintained;
mdBook is stable and Rust-native — decision is final, don't re-propose
oranda). Companion to `review-handoff.md`, whose addendum lists what's already
done. Ordered so each stage is useful on its own.

> **Status (2026-07-23):** stages 0, 1, and 2 are **done and committed** on
> `main`, except for the four steps that are Anders' to take: `git push`,
> tagging `v0.1.4`, enabling GitHub Pages, and the crates.io publish. See the
> per-stage notes below and the addendum in `review-handoff.md`.
>
> **`stages-0-2-handoff.md` is the review request** — what was found that this
> plan didn't anticipate, and the four places the execution departed from it.

---

## Stage 0 — land what exists (blocking everything else)

**Done**, except push/tag. Commits: `054c26d` (rustfmt config), `3ae3d56` (repo
URL), `3219e21` (notes), `6ad6f04` (changelog + 0.1.4 bump).

Two things this stage turned up that the plan didn't anticipate:

- **CI's first run was already red.** `cargo fmt --check` failed on the runner:
  the repo had no `rustfmt.toml`, so the style came from a machine-global config
  locally and from rustfmt's defaults in CI. Now checked in, stable-only keys,
  stable and nightly agree.
- **`repository`/`homepage` pointed at a URL that 404s** (`Rottschaferanders`;
  the repo is under `rootschafer`). Fixed before it could reach crates.io.

`cargo publish --dry-run` is clean. `dist` picks up `CHANGELOG.md` on its own —
it ships in every archive and the matching section becomes the release body.

1. Commit the pending work in logical commits: the polish pass (+ the
   `toml = "1"` bump), the `/run-rustsweep` skill, the notes. Tree is
   verified green: 18 unit + 46 integration tests, clippy silent,
   `cargo fmt --check` clean, `dist plan` OK, smoke driver passes.
2. Push and watch the new `ci.yml` actually go green on GitHub — a
   workflow's first run is the only real test of it.
3. `CHANGELOG.md` (Keep a Changelog format), reconstructed from git history
   while it's still short. First public release should ship with one.
4. Version bump → `0.1.4`, tag `v0.1.4`, push the tag → first dist release:
   prebuilt binaries + shell/powershell installers. Copy the installer
   one-liner from the release page into the README's Install section.
5. **crates.io publish is Anders' decision** — publishing claims the name
   (unclaimed as of 2026-07-22, not reserved). `cargo publish --dry-run`
   to rehearse; don't publish without an explicit go-ahead.

## Stage 1 — drift-proof CLI docs (the foundation, no website yet)

**Done** — commit `dcc42b0`. All four items landed as specified, in
`tests/docs.rs` (3 tests) plus `cli::docs_command()`. Deltas worth recording:

- The README block renders **short** help (`-h`), not long. Long help puts each
  option's full paragraph on its own lines — 60 lines for eleven flags, too much
  for a front door, and its blank continuation lines carry trailing whitespace
  that any editor would strip and break the guard on. The long form's content is
  in the book's CLI reference instead.
- **Wrapping had to be pinned from both ends** or the golden test would depend
  on the terminal that ran it: `max_term_width = 100` on the command and
  `term_width(100)` in `docs_command`. Needed clap's `wrap_help` feature.
- **The man page embeds the version**, so a release bump requires a re-bless.
  Added to the README's release steps rather than engineered around — the man
  page should carry the version.
- The stale-docs message is written to the stderr fd directly, since the harness
  loses captured output when a failing assertion aborts on macOS (§3 of the
  handoff). Verified by hand that the guidance shows.

Principle: **the clap derive structs are the only source of truth.** Every
doc artifact is generated from `Cli::command()` or guarded by a test that
diffs against it. No hand-maintained copies.

1. **Golden test for the README usage block.** A unit test renders the help
   (`Cli::command()`, `render_long_help()` or equivalent) and asserts the
   README's fenced usage block matches; running with `UPDATE_DOCS=1` makes
   the same test rewrite the block in place (the "bless" pattern). Zero new
   dependencies; kills the drift hazard that exists today.
2. **Markdown CLI reference** → `docs/src/cli-reference.md` via
   `clap-markdown`, guarded/blessed by the same test. This lands directly
   inside the mdBook source tree (stage 2) so the site's reference page can
   never drift either.
3. **Man page** via `clap_mangen` → committed `docs/man/rustsweep.1`, same
   bless guard, added to release archives via the `include` key in
   `dist-workspace.toml`.
4. **Shell completions** via `clap_complete`, exposed as a hidden
   `--completions <shell>` flag on the binary — always current by
   construction, nothing to generate or ship. One README/book paragraph
   showing `rustsweep --completions zsh`.

Deliberately *not* an `xtask` workspace: at this size the bless-style tests
double as the generator and the repo stays single-package.

## Stage 2 — the mdBook site on github.io

**Done** — commit `e85bb3e`. Exactly the chapter layout below, built with mdBook
0.5.4 (pinned in the workflow). `{{#include ../../config.example.toml}}` works
as hoped — 0.5.4 allows an include that escapes the book root. All internal
links and anchors were checked against the built HTML.

Deltas: `docs.yml` also builds (without deploying) on pull requests, so a broken
`SUMMARY.md` or dangling include fails in review rather than on `main`.
`tests/docs.rs` had to join `docs/` in Cargo.toml's `exclude` — it diffs against
files the package doesn't ship, so a packaged crate would otherwise carry a test
that cannot pass.

**Not done, and only Anders can do it: repo Settings → Pages → Source "GitHub
Actions".** The deploy job 404s until that's set.

Layout — book lives under `docs/`, so the repo root stays clean:

```
docs/
  book.toml            # title, authors, git-repository-url, edit-url-template
  src/
    SUMMARY.md
    introduction.md    # what/why, the one-paragraph pitch + safety story
    installation.md    # installer one-liners (from the v0.1.4 release), cargo install, binaries
    quick-start.md     # the --dry-run --show-size first run, reading the output, saying y/n
    cli-reference.md   # AUTOGENERATED (stage 1.2) — never hand-edit
    configuration.md   # config.toml keys, precedence, the no-`yes`-key rationale
    ignore-patterns.md # bare names vs globs, cost model, root-exemption rule
    filters.md         # --keep-days/--keep-size polarity, apparent sizes, ≥ bounds
    how-it-decides.md  # the CACHEDIR.TAG invariant — why it never deletes the wrong thing
    faq.md
```

Mechanics and drift-proofing:

- `mdbook build docs` locally; `mdbook serve docs` to iterate.
- Use mdBook's `{{#include ../../config.example.toml}}` to embed the real
  example config in `configuration.md` instead of copying it — same trick
  anywhere the book would otherwise duplicate a repo file.
- Content strategy vs README: the README stays the short front door (pitch,
  install, one example, link to the book); depth migrates to the book. The
  autogenerated usage block stays in both, kept honest by the stage-1 test.
- Deploy: `.github/workflows/docs.yml` — on push to `main`: install mdbook
  (pinned version, via prebuilt binary download or `cargo-binstall`), run
  `mdbook build docs`, then `actions/upload-pages-artifact` +
  `actions/deploy-pages`. Keep it a separate workflow from ci.yml so docs
  deploys never block or slow test runs.
- **One-time manual step for Anders:** repo Settings → Pages → Source:
  "GitHub Actions". Site lands at `rootschafer.github.io/rustsweep`.
- Add `docs/book/` (build output) to `.gitignore`; add `docs/` to
  Cargo.toml `exclude` so it stays out of the crates.io package.
- What we lose vs oranda is the auto-generated platform-detecting install
  widget; compensate with a prominent, hand-written install page showing
  the dist one-liners (they're stable per-release URLs via `/latest/`).

## Stage 3 — deepen, only on demand

The stage-2 skeleton ships with short-but-real chapters. Grow a chapter when
its question actually comes up (an issue, a confused user, a second ask):
recipes (cron/CI usage, pre-backup sweeps), a "why didn't it delete X"
walkthrough, packaging pages (homebrew) as those channels appear.

## Also worth doing for "all users" (unordered)

- **Windows CI job.** Windows users get release binaries today but tests
  never run there. Blocker is only the `expand_tilde` unit test expecting
  `$HOME`; give the test the same `HOME`-else-`USERPROFILE` fallback the
  code already has, then add `windows-latest` to the ci.yml matrix.
- **Homebrew tap** via dist (`installers = [… "homebrew"]` + a
  `homebrew-tap` repo). Cheap once releases flow; big reach for Mac users.
- Rename `LICENSE.txt` → `LICENSE` (pure convention; everything detects both).

## Decisions log

- **Docs site: mdBook, not oranda** (Anders, 2026-07-23) — oranda appears
  unmaintained; mdBook chosen for stability and the Rust-native feel.
- **License: MIT** (Cargo.toml aligned to the committed LICENSE.txt,
  2026-07-22) — revisit only if dual MIT/Apache-2.0 was actually intended.
- **Walk root is exempt from the ignore list** — documented + tested.
- **`rustfmt.toml` is checked in, stable-only** (2026-07-23) — CI's first run
  failed on `cargo fmt --check` because the style was only ever in a
  contributor's machine-global config. Unstable keys are warned about rather
  than applied by stable rustfmt, so keeping any would recreate the split.
- **The README's usage block is short help, not long** (2026-07-23) — the long
  form is 60 lines and its blank continuation lines carry trailing whitespace an
  editor would strip, breaking the guard. Long help's content lives in the
  book's CLI reference.
- **No `xtask`** (as planned) — the bless-style tests are the generator, and the
  repo stays a single package.
- **Version lives in the man page** (2026-07-23) — accepted the re-bless step at
  release time rather than stripping it; documented in the README.

### Still open, all Anders'

- **`git push`** — five commits ahead of `origin/main`. CI has never run green;
  this is the first push that should.
- **Tag `v0.1.4` and push it** — triggers a public release.
- **Settings → Pages → Source: "GitHub Actions"** — one-time, manual, and the
  docs deploy fails until it's done.
- **crates.io publish** — `cargo publish --dry-run` is clean; the name was
  unclaimed as of 2026-07-22, which is not a reservation.
- Homebrew tap, Windows CI job (see "Also worth doing" above).
