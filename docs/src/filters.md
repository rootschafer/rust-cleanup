# Filters

`--keep-days` and `--keep-size` narrow a run to the build directories actually
worth reclaiming. Neither is on by default, and a run without either pays
nothing for them.

## Read the polarity carefully

Both flags are named for what they **keep**, not what they clean. This is the
one thing about them that reliably catches people out, so:

| Flag | Keeps | Cleans |
| --- | --- | --- |
| `--keep-days 30` | build dirs touched **within** the last 30 days | ones untouched for 30 days or more |
| `--keep-size 500MB` | build dirs **smaller** than 500 MB | ones at least 500 MB |

`--keep-days` protects work in progress. `--keep-size` skips directories too
small for the deletion to be worth the rebuild.

Set both and a directory has to pass **both** to be cleaned — it must be old
*and* big:

```sh
rustsweep --path ~/Code --keep-days 30 --keep-size 500MB
```

Both bounds are inclusive on the clean side: exactly 500 MB is cleaned, and a
directory whose newest file is exactly 30 days old is cleaned.

At the end of the run, anything held back is counted:

```console
14 build dir(s) kept by --keep-days/--keep-size.
```

## Size syntax

`--keep-size` accepts a raw byte count or a human size. Units are **1024-based
throughout** — `500MB` and `500MiB` mean the same 524,288,000 bytes. The `b`/`ib`
suffix is optional and case doesn't matter, so `1G`, `1GB`, `1gib` are all one
gibibyte. Fractions work: `1.5G`.

An invalid value on the command line is an error and stops the run. The same
value in the config file warns, is treated as unset, and the run continues —
consistent with how the rest of the config handles a bad value.

## What "touched" means

`--keep-days` uses the **newest file mtime anywhere in the build directory**,
not the directory's own mtime. Building a project rewrites files deep inside
`target/`, which a plain `stat` on `target/` won't always reflect.

Clock skew into the future counts as recent, so a file with a timestamp ahead of
now protects its directory rather than being read as ancient.

## <a id="apparent-sizes"></a>Apparent sizes

Sizes are **apparent** sizes — the sum of file lengths — not the number of disk
blocks allocated. This applies to `--keep-size`, to `--show-size`, and to the
"Freed ~X" total.

`du` reports allocated blocks by default, so totals will differ a little. On a
filesystem with compression or deduplication (APFS, Btrfs, ZFS) they can differ
by a lot, and rustsweep's number will read high. `du --apparent-size` is the
comparable measurement.

## What happens when a directory can't be fully read

Measuring walks the entire build directory. If any part of it fails to read —
permissions, a file that vanished mid-walk, an I/O error — the measurement is
marked incomplete, and:

- with `--show-size`, the size is shown as a lower bound: `(≥ 1.2 GiB)`;
- with `--keep-days` or `--keep-size` set, the directory is **kept**.

The reasoning: an unreadable subtree could be arbitrarily large or arbitrarily
recent, so partial numbers are never grounds for deleting. Incomplete
information doesn't count as permission.

Note that this only applies when a filter is set. `--show-size` on its own is
reporting, not a decision, so a directory that couldn't be fully measured is
still offered — with its size marked as the lower bound it is.

## The cost

`--show-size`, `--keep-days`, and `--keep-size` all require the same thing:
walking every discovered build directory in full. That is the expensive part of
using them, and it is unavoidable — the numbers aren't stored anywhere.

A run with none of the three never walks inside a build directory at all. It
sees the `CACHEDIR.TAG`, records the directory, and stops. That is why the
default run is fast, and why turning on a filter is a real, visible cost on a
large tree.
