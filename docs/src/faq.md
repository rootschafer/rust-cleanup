# FAQ

### Why didn't it clean project X?

Work down this list:

1. **It was already clean.** No build directory, nothing to do. Counted in the
   `N project(s) were already clean` line.
2. **A filter kept it.** With `--keep-days`/`--keep-size` set, check the
   `N build dir(s) kept by --keep-days/--keep-size` line, and re-read
   [the polarity](filters.md#read-the-polarity-carefully) — the flags name what
   they keep.
3. **It's under an ignored directory.** Anything matching your ignore list, or
   the built-ins `.git`/`node_modules`/`.jj`, is never scanned. Try
   `rustsweep --path <the project itself> --dry-run`: the search root is exempt
   from the ignore list, so if that finds it, the ignore list was the reason.
4. **`--max-depth` cut it off**, or it was behind a symlink and
   `--follow-symlinks` wasn't set.
5. **`cargo metadata` couldn't read it.** Check the
   `N project(s) couldn't be read` line and re-run with `-v`. Its build
   directory should still have been found by the direct scan as a stray.
6. **Its build directory has no cargo `CACHEDIR.TAG`.** Very old build
   directories predate the tag. If cargo can still resolve the project,
   `cargo clean` handles it anyway; a detached one won't be found.

### Why is it reporting a build directory in a project I deleted?

That's the orphan case, and it's exactly what `--orphans` is for. rustsweep
won't remove it on its own because it can't tell your live `--target-dir` from
an abandoned one. See
[the classification](how-it-decides.md#stage-3--the-classification).

### The sizes don't match `du`.

Expected. rustsweep reports apparent sizes (the sum of file lengths); `du`
reports allocated blocks. Compare against `du --apparent-size`, and see
[apparent sizes](filters.md#apparent-sizes). On a compressing or deduplicating
filesystem the gap can be large.

### What's the `≥` in front of a size?

Part of that build directory couldn't be read, so the number is a lower bound.
With a filter set, such a directory is kept rather than cleaned —
[details](filters.md#what-happens-when-a-directory-cant-be-fully-read).

### Will it delete a directory named `target` that isn't a build directory?

No. Names are not what it matches on; a cargo-authored `CACHEDIR.TAG` is. See
[the invariant](how-it-decides.md#the-invariant).

### Will it delete another tool's cache?

No. Many tools write a `CACHEDIR.TAG` — it's a shared convention — but the file's
body has to mention cargo. rustsweep reads the body.

### Can I put `yes = true` in the config file?

No, and it's rejected rather than ignored. Cleaning without a prompt is the one
irreversible thing here, so it has to be typed per run.
[Why](configuration.md#there-is-no-yes-key).

### How do I turn off something my config file turned on, for one run?

There are no `--no-*` flags today. Point `$RUSTSWEEP_CONFIG` at a file that
doesn't exist to run with the config out of the way entirely:

```sh
RUSTSWEEP_CONFIG=/nonexistent rustsweep --path ~/Code --dry-run
```

If you find yourself doing that often, the `--no-*` flags are a deferred idea
rather than a rejected one — say so in an issue.

### Why is the scan slower than it was?

Almost always one of two things:

- **A glob in your ignore list.** A single glob pattern makes every directory in
  the walk resolve its absolute path. Roughly 45% slower on a large tree. Prefer
  bare names — [why](ignore-patterns.md#why-bare-names-are-worth-preferring).
- **`--show-size`, `--keep-days`, or `--keep-size`.** Any of the three walks
  every build directory in full. [The cost](filters.md#the-cost).

### Is it safe to run in CI or from cron?

Yes, with `--yes`, and read [what this does not protect
against](how-it-decides.md#what-this-does-not-protect-against) first. Without
`--yes`, a non-interactive run reaches a prompt, gets EOF, and answers "no" to
everything — safe, but it won't clean anything either.

### Does it need cargo installed?

Yes. rustsweep shells out to `cargo metadata` to resolve projects and to
`cargo clean` to clean them. The prebuilt binaries don't need a Rust toolchain
to *install*, but they need one to run.

### Where do I report something?

[github.com/rootschafer/rustsweep/issues](https://github.com/rootschafer/rustsweep/issues).
Output from a `--dry-run --show-size -v` run is the most useful thing to attach.
