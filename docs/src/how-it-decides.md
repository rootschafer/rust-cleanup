# How it decides what to delete

This chapter is the answer to "why should I trust this with `--yes`?". It walks
the whole decision, in the order the program makes it.

## The invariant

> A directory is a Cargo build directory **if and only if** it contains a
> `CACHEDIR.TAG` file whose body contains the word "cargo".

Nothing else qualifies a directory for deletion. Not its name, not its
location, not its contents.

`CACHEDIR.TAG` is a [shared convention](https://bford.info/cachedir/) for
marking a directory as regenerable cache, and cargo writes one into every build
directory it creates, with a body reading `# This file is a cache directory tag
created by cargo.`

Two properties follow, and they're the reason the test is shaped this way:

**It can't match the wrong thing.** A directory you named `target` for your own
reasons has no cargo tag, so it is invisible to the scan. Another tool's cache
*does* often carry a `CACHEDIR.TAG` — that's the convention working as intended
— but its body won't mention cargo, so it is skipped too. The scan reads the
body, not just the filename.

**It can't miss the right thing.** A build directory that has been renamed,
relocated by `--target-dir`, or produced under an old `build.target-dir` setting
keeps its tag, so it is still found. This is the class of leftover `cargo clean`
has no way to reach: cargo only knows about the build directory the current
manifest resolves to.

## Stage 1 — the walk

One parallel pass over the tree under `--path`, reading each directory once.
Both markers — `Cargo.toml` and `CACHEDIR.TAG` — are detected from the directory
listing that pass already has, so there is no extra filesystem call per
candidate.

For each directory:

- **Ignored?** ([ignore patterns](ignore-patterns.md)) → prune the subtree.
  The search root itself is exempt from this.
- **Holds a `CACHEDIR.TAG`?** → don't descend, ever. If the tag's body mentions
  cargo, record the directory as a build directory. If it doesn't, record
  nothing. Either way the walk stops there: a cache directory holds no projects.
- **Holds a `Cargo.toml`?** → record it as a project, and *keep descending* —
  workspaces contain member crates, and crates contain their own nested crates
  (`fuzz/`, `xtask/`, crate-shaped examples).

Symlinked directories are not followed unless `--follow-symlinks` is passed, so
the search can't escape the tree you pointed it at. With that flag on, a
visited-set of canonical paths guards against cycles; `--max-depth` bounds the
traversal either way.

The walk yields two lists: projects, and build directories.

## Stage 2 — the plan

Each project gets asked `cargo metadata` for the authoritative answer to "what
is your workspace root, and what is your build directory?". Cargo's answer is
what gets used — that's how a project with a relocated build directory is still
cleaned correctly.

Workspaces are folded into a single job, so a workspace with forty members is
cleaned once, not forty times. Workspace roots are resolved first (identified by
a cheap scan for a `[workspace]` line, no cargo process spawned), which is what
lets members be recognized as already covered.

A project `cargo metadata` can't read — a broken manifest, a crate detached from
the workspace that encloses it — is not fatal. It's collected, reported at the
end, and listed individually with `-v`. Its build directory is still handled by
the direct scan, so nothing is silently missed.

## Stage 3 — the classification

Every discovered build directory now falls into exactly one of three cases:

**It is some project's resolved build directory.** → Clean it by running
`cargo clean` in that project. rustsweep does not delete the directory itself
here; cargo does its own removal, and its `Removed N files` line is cargo's.
Deduplicated by resolved build directory, so projects sharing one are handled
once.

**It sits inside a known project but isn't that project's current build
directory.** → A stray: a renamed `target/`, or a `--target-dir` leftover.
`cargo clean` cannot reach it, so it is removed directly. The project it sits
inside is named in the prompt, so you can see what you're being asked about.

**It sits inside no known project at all.** → An orphan. This is the ambiguous
case: it might be a `--target-dir` you still use, or the remains of a project
deleted years ago. rustsweep **counts these and tells you, but does not touch
them** unless you pass `--orphans`.

Every one of those three paths funnels through a single function. There is
exactly one place in the program where a deletion happens, which is what makes
the guarantees in this chapter checkable rather than aspirational.

## Stage 4 — the gates

Before anything is removed, in order:

1. **Filters.** With `--keep-days` or `--keep-size` set, the directory is
   measured and judged. A directory that couldn't be measured completely is
   kept. See [Filters](filters.md).
2. **`--dry-run`.** Print the decision, count it toward the projected total,
   act on nothing. Every line a real run would produce, produced without
   consequence.
3. **The prompt.** `y`/`yes` proceeds, `n`/`no` skips and lists the path under
   "Skipped", anything else re-asks. A closed stdin (EOF) or a read error
   answers **no** — a run in a pipeline can't delete by accident.
4. **`--yes`** bypasses step 3 and nothing else. Filters and `--dry-run` still
   apply. It is command-line only; see
   [there is no `yes` key](configuration.md#there-is-no-yes-key).

The "Freed ~X" total counts only removals that actually succeeded. A failed
`cargo clean` or a failed directory removal prints an error and does not inflate
the total.

## What this does not protect against

Honesty about the edges:

- **It is still a deletion tool.** `--yes` at the wrong `--path` will clean
  every Cargo project underneath it. The build outputs are regenerable by
  definition, but regenerating them costs time.
- **`cargo clean` removes the whole build directory**, including anything you
  put inside `target/` yourself. That's cargo's behavior, not rustsweep's, but
  it applies all the same.
- **A file inside a build directory that isn't regenerable is gone.** The
  `CACHEDIR.TAG` contract is a promise from cargo that the directory's contents
  are cache. Anything you stored there against that promise is not exempt.
