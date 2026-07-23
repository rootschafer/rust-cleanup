# Quick start

## The first run

Always start here. `--dry-run` makes every decision and prints it, then deletes
nothing; `--show-size` measures each build directory so you can see what the
run is actually worth.

```sh
rustsweep --path ~/Code --dry-run --show-size
```

```console
Scanned 61034 directories: found 1412 project(s) and 388 build dir(s).
Dry run — nothing will be deleted.
Would clean ~/Code/sensor-fw — build dir ~/Code/sensor-fw/target (800.1 KiB)
Would clean ~/Code/hyperloop — build dir ~/Code/hyperloop/target (3.1 GiB)
Would remove stray build dir ~/Code/old-experiment/target-old (1.5 GiB) (inside ~/Code/old-experiment)
Would free ~4.6 GiB.
1 project(s) were already clean.
Found 1 orphaned Cargo build dir(s) not tied to any project; re-run with --orphans to remove them.
```

## Reading the output

**`Scanned N directories: found P project(s) and B build dir(s).`** — one line
per run, printed after the walk. `P` counts directories holding a `Cargo.toml`;
`B` counts directories carrying cargo's `CACHEDIR.TAG`. `B` is usually smaller
than `P`, because workspace members share one build directory and clean projects
have none.

Then one line per thing it would act on, in three flavors:

| Line | What it is |
| --- | --- |
| `Would clean <project> — build dir <dir>` | A project's current build directory, the one `cargo metadata` resolved. It gets cleaned by running `cargo clean` in the project, not by deleting the directory. |
| `Would remove stray build dir <dir> (inside <project>)` | A cargo build directory sitting inside a project but *not* that project's current build directory — a renamed `target/`, or a leftover from `--target-dir`. `cargo clean` cannot reach these. Removed directly. |
| `Would remove orphaned build dir <dir>` | A cargo build directory with no project around it at all. Only ever shown or touched with [`--orphans`](#orphans). |

And a summary at the end:

- **`Would free ~X`** / **`Freed ~X`** — only printed with `--show-size`. See
  [apparent sizes](filters.md#apparent-sizes) for why this can differ from `du`.
- **`N project(s) were already clean`** — they had no build directory, so there
  was nothing to do.
- **`N build dir(s) kept by --keep-days/--keep-size`** — see [Filters](filters.md).
- **`N project(s) couldn't be read by cargo metadata`** — broken or detached
  manifests. Their build directories are still found by the direct scan, so
  nothing is missed; `-v` lists which ones and why.
- **`Found N orphaned Cargo build dir(s)…`** — the nudge toward `--orphans`.
- **`Skipped:`** — everything you answered `n` to.

## Doing it for real

Drop `--dry-run` and you get asked about each one:

```console
$ rustsweep --path ~/Code --show-size
Scanned 61034 directories: found 1412 project(s) and 388 build dir(s).
~/Code/sensor-fw is a Cargo project (build dir: ~/Code/sensor-fw/target (800.1 KiB)). Clean it? (y/n): y
     Removed 3 files, 800.5KiB total
~/Code/hyperloop is a Cargo project (build dir: ~/Code/hyperloop/target (3.1 GiB)). Clean it? (y/n): n
~/Code/old-experiment/target-old (1.5 GiB) is a stray Cargo build dir (not ~/Code/old-experiment's current build dir). Remove it? (y/n): n
Freed ~800.1 KiB.
Skipped:
  ~/Code/hyperloop
  ~/Code/old-experiment/target-old
```

Only `y`/`yes` and `n`/`no` are accepted; anything else re-asks. Closing stdin
(EOF) answers **no**, so a run in a pipeline can't delete anything by accident.
The `Removed 3 files…` line comes from `cargo clean` itself.

To skip the prompts entirely:

```sh
rustsweep --path ~/Code --yes
```

`--yes` is command-line only, on purpose — it is deliberately not a config key,
so the irreversible mode always has to be typed. See
[Configuration](configuration.md#there-is-no-yes-key).

## <a id="orphans"></a>Orphans

A build directory with no project around it is ambiguous: it might be a
`--target-dir` you still use, or it might be a project you deleted three years
ago. rustsweep reports these but leaves them alone unless you ask:

```sh
rustsweep --path ~/Code --dry-run --show-size --orphans
```

```console
Would remove orphaned build dir ~/Code/scratch-build (2.3 GiB)
```

## A safer recurring run

Once you trust it, the combination worth putting in a shell alias is a filtered
one — clean the big, cold build directories and leave everything you're actively
working on alone:

```sh
rustsweep --path ~/Code --keep-days 30 --keep-size 500MB --yes
```

That cleans build directories that are both untouched for 30 days *and* at least
500 MB. [Filters](filters.md) covers the polarity of those two flags, which is
the one thing about them that catches people out.
