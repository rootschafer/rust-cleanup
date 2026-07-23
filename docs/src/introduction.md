# Introduction

Rust build artifacts are enormous and they never go away on their own. A single
`target/` directory routinely runs to gigabytes, and if you have a few hundred
checkouts on disk, most of that space is holding output for projects you last
touched a year ago.

`cargo clean` fixes one project at a time, and only if you remember where the
project is. **rustsweep** points at a directory, finds every Cargo project
beneath it, cleans each workspace exactly once, and — this is the part
`cargo clean` genuinely cannot do — also removes the build directories that no
longer belong to any project cargo can see: a renamed `target/`, a directory
left behind by `cargo build --target-dir`, or leftovers from an old
`build.target-dir` setting.

```console
$ rustsweep --path ~/Code --dry-run --show-size
Scanned 61034 directories: found 1412 project(s) and 388 build dir(s).
Dry run — nothing will be deleted.
Would clean /Users/you/Code/Rust/thing — build dir /Users/you/Code/Rust/thing/target (3.1 GiB)
…
Would free ~214.7 GiB.
```

## The safety story

A tool that deletes directories in bulk has exactly one job beyond deleting
them: never delete the wrong one. rustsweep's answer is a single narrow test.

> A directory is a Cargo build directory **if and only if** it contains a
> `CACHEDIR.TAG` file whose body mentions cargo.

That is the tag cargo itself writes into every build directory. Keying on it,
rather than on the name `target`, means:

- a directory you named `target` for your own reasons is never touched;
- another tool's cache — which may well have its own `CACHEDIR.TAG`, since the
  format is a shared convention — is never touched, because the body won't say
  cargo;
- a build directory that has been *renamed* or relocated is still found.

On top of that:

- **Nothing is deleted without a `y`** unless you pass `--yes`, and `--yes`
  cannot be set in the config file. It has to be typed, per run.
- `--dry-run` prints the exact same decisions without acting on any of them.
- Ignore patterns can only ever *add* protection. `--ignore` extends the config
  file's list rather than replacing it, and the built-in floor (`.git`,
  `node_modules`, `.jj`) can't be removed.
- With `--keep-days` or `--keep-size` set, a build directory that couldn't be
  fully measured is kept, not deleted. Incomplete information never counts as
  permission.

[How it decides what to delete](how-it-decides.md) walks through the whole
decision path if you want to see it end to end.

## Where to go next

- [Installation](installation.md) — a one-line installer, `cargo install`, or a
  prebuilt binary.
- [Quick start](quick-start.md) — the first run, and how to read what it prints.
- [CLI reference](cli-reference.md) — every flag, generated from the source.
