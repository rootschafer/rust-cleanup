# Handoff: stages 0–2 of `docs-and-release-plan.md`, executed

**Written:** 2026-07-23, back to whoever wrote `docs-and-release-plan.md`.
**Scope:** stages 0, 1, and 2 of that plan, in order.
**State:** 8 commits on `main`, **not pushed**. Working tree clean.
27 files, +2336/−65.

This is a review request, not a summary. The parts worth your attention are
§2 (things the plan didn't know about) and §3 (where I departed from the plan
and why) — everything else went as written.

---

## 0. Verify it in one command

```sh
cargo test && cargo clippy --all-targets && cargo fmt --check \
  && dist plan && mdbook build docs \
  && .claude/skills/run-rustsweep/smoke.sh
```

18 unit + 48 integration + 3 docs tests; clippy silent; `SMOKE OK`.
`cargo publish --dry-run` is also clean.

I ran that gate before each commit, and then once more **in a fresh clone**
under the `stable` toolchain. The fresh-clone run is the one that matters here
— see §2.1 for why my own working tree was not a trustworthy place to check
formatting.

## 1. The commits

| Commit | What |
| --- | --- |
| `054c26d` | Check in `rustfmt.toml` so CI agrees with local |
| `3ae3d56` | Point package metadata and README at the real repo URL |
| `3219e21` | Check in the development notes |
| `6ad6f04` | `CHANGELOG.md` + version bump to 0.1.4 |
| `dcc42b0` | **Stage 1** — generate the CLI docs from the clap definitions |
| `e85bb3e` | **Stage 2** — the mdBook site |
| `5a2714e` | Update `review-handoff.md` and the plan's status/decisions log |
| `86d7da1` | Fix the changelog compare links and the Pages action pin |

New dependencies: `clap_complete` (runtime), `clap_mangen` + `clap-markdown`
(dev), and clap's `wrap_help` feature (pulls `terminal_size`). Nothing else.

## 2. Things the plan didn't anticipate

These are the findings. If you only read one section, read this one.

### 2.1 CI was already red, for a reason invisible from the working tree

Stage 0.2 says "watch the new `ci.yml` actually go green — a workflow's first
run is the only real test of it." Its first run had **already happened and
failed**, on `cargo fmt --check`.

Cause: the repo had no `rustfmt.toml`. The tabs-and-120-columns style was
coming from a machine-global config at
`~/Library/Application Support/rustfmt/rustfmt.toml`, so `cargo fmt --check`
passed locally and failed on a runner that had never seen that file. The
handoff's §4 "verify with" list is exactly the gate that could not catch this,
because every command in it was being run in the environment that hid the
problem.

Fixed by checking the style in, **stable-only keys**: CI runs stable rustfmt,
and unstable keys are warned about rather than applied, so keeping any would
recreate the same split in a subtler form. Dropping the nightly-only
`short_array_element_width_threshold` is what reflowed the arg-id array in
`cli.rs::tests` to one-per-line. Stable and nightly now produce byte-identical
output.

**The generalizable bit:** the local gate is only as good as the environment
running it. Everything I verified afterward, I verified in a fresh clone too.

### 2.2 The package metadata pointed at a repo that does not exist

`repository` and `homepage` in `Cargo.toml` were
`github.com/Rottschaferanders/rustsweep`. That 404s. The repo is
`github.com/rootschafer/rustsweep` (confirmed against the git remote and
`gh api user`). The README's clone command had the same typo, and the plan's
own stage-2 text predicted the site at `rottschaferanders.github.io`.

crates.io renders `repository` verbatim, so this would have shipped a dead link
on the package page. Caught before the publish decision rather than after, but
it was one `cargo publish` away from being permanent for that version.

### 2.3 `dist` picks up `CHANGELOG.md` by itself

No config change needed. Once the file existed, `dist plan` started shipping it
in every archive and using the section matching the tag as the GitHub Release
body. Worth knowing because it makes the changelog load-bearing at release time
rather than decorative: **a release with no matching section gets an empty
release body.**

### 2.4 `tests/docs.rs` had to be excluded from the package alongside `docs/`

Stage 2 says to exclude `docs/` from the crates.io package. Doing only that
leaves a packaged crate carrying a test that reads `docs/src/cli-reference.md`
and `docs/man/rustsweep.1` — files the package no longer ships. `cargo publish`
does not run tests, so the dry run stayed green and this would have gone
unnoticed until someone ran `cargo test` on the published crate.

`exclude` now lists `tests/docs.rs` too, with the reasoning in a comment.

### 2.5 The man page embeds the version

`clap_mangen` writes `.TH rustsweep 1 "rustsweep 0.1.4"`. So a version bump
without a re-bless turns CI red. I took that as correct — a man page should
carry its version — and added `UPDATE_DOCS=1 cargo test` to the README's
release steps rather than engineering it away. **Flagging it because it's a new
way for a release to fail, and it fails at exactly the moment you're least
inclined to read the error.**

## 3. Where I departed from the plan

Four judgment calls. Each is cheap to reverse if you disagree.

### 3.1 The README block is short help (`-h`), not long (`--help`)

Stage 1.1 says "renders the help (`render_long_help()` or equivalent)". I used
`render_help()`.

Two reasons. The long form is 60 lines for eleven flags — too much for a file
the plan itself wants to be "the short front door". More concretely, long help
puts blank continuation lines between option paragraphs, and those lines carry
**trailing whitespace**; any editor or pre-commit hook that strips it silently
breaks the golden test, which is a bad failure mode for a guard whose whole
purpose is to be trustworthy.

The long form's content is not lost — it's what `clap-markdown` renders into
`docs/src/cli-reference.md`, where the width isn't a problem.

### 3.2 Wrapping is pinned from both ends

Not in the plan, but the golden test is unsound without it: `render_help()`
consults terminal width, so the test would pass or fail depending on the
terminal that ran it. `max_term_width = 100` on the command **and**
`term_width(100)` in `cli::docs_command()`. This is why clap's `wrap_help`
feature is now on — it's a real behavior change for users (help now wraps at
100 columns instead of running long), and I think an improvement, but it is a
change.

### 3.3 `docs.yml` also builds on pull requests

The plan says "on push to `main`". I added `pull_request` as a build-only job,
no deploy. A broken `SUMMARY.md` or a dangling `{{#include}}` now fails in
review instead of on `main`. This is why the `concurrency: pages` group sits on
the deploy job rather than the workflow — at workflow level it would make PR
builds queue behind deploys for no benefit.

### 3.4 Two small fixes not in any stage

`rustfmt.toml` (§2.1) and the repo URL (§2.2). Both were blocking the stage
they sat in — you cannot watch CI go green when it is red for an unrelated
reason, and you should not publish metadata with a dead link — so I fixed them
rather than reporting and stopping. Both are their own commits, easy to isolate.

## 4. Verified by doing, not by reasoning

I initially reported this work as done on the strength of the local gate, was
asked whether I was sure, and found three defects on the re-audit. Recording
what I actually exercised, so the next reader knows which claims are load-bearing:

- **`dist build`** produces a real archive; listed the tar and confirmed
  `rustsweep.1` is inside. (`dist plan` alone does not prove the `include` path
  resolves.)
- **The man page renders** under `man ./docs/man/rustsweep.1`.
- **The mdbook tarball** has a bare `mdbook` at its root, so the workflow's
  `tar -xz -C ~/.local/bin` really does put it on `PATH`.
- **Every internal book link and anchor** checked against the built HTML,
  including the hand-written `<a id="…">` anchors.
- **The golden test actually fails when the docs drift** — corrupted the README
  block on purpose and watched it fail, then confirmed the "run `UPDATE_DOCS=1`"
  guidance survives the macOS abort (it's written to the stderr fd directly,
  because the harness's captured output is lost along with the panic message —
  §3 of `review-handoff.md`).
- **A fresh clone passes the whole gate** under stable.

The three defects that re-audit caught, all in `86d7da1`: two wrong
`CHANGELOG.md` compare ranges, and `deploy-pages` pinned to v4 where GitHub's
own Pages starter now pairs `upload-pages-artifact@v3` with `deploy-pages@v5`.

## 5. What I did not touch

- **Stage 3** — correctly gated on demand; nothing to do.
- **Windows CI job, Homebrew tap, `LICENSE.txt` → `LICENSE`** — the plan's
  "also worth doing", not stages 0–2. The Windows blocker named in the plan
  (`expand_tilde`'s `$HOME` assumption) is still exactly as described.
- **`plan.rs` scaling** (§5.4 of the handoff) — still untouched, per its
  "measure first" note.
- **`--no-<flag>` overrides** — still deferred. The FAQ now documents the
  `RUSTSWEEP_CONFIG=/nonexistent` workaround and says the flags are a deferred
  idea rather than a rejected one.
- **Anything outward-facing.** See §6.

## 6. Blocked on Anders, in this order

1. **`git push`** — 8 commits. This is the first push that should turn CI green.
2. **Settings → Pages → Source: "GitHub Actions"** — one-time, manual, only the
   repo owner can do it. The deploy job 404s until then.
3. **Tag `v0.1.4` and push the tag** — triggers a public release. Worth doing
   only after 1 confirms green.
4. **crates.io publish** — dry run is clean; the name was unclaimed as of
   2026-07-22, which is not a reservation.

The order matters: 3 is the irreversible one, and 1 is what tells you whether
it will work.

## 7. Known-unverified

**Neither workflow has ever run in the state I'm handing over.** `ci.yml` has
one historical failed run (§2.1); `docs.yml` has never run at all. I checked
what can be checked offline — YAML parses, every pinned action tag exists, the
mdbook download URL and archive layout are right, the book builds — but the
first real run is still the first real run. If `docs.yml` fails, the two
candidates in order of likelihood are the Pages setting in §6.2 not being set,
and the `~/.local/bin` PATH step.

Everything else in §4 I exercised directly.
