# Ignore patterns

An ignored directory is never *descended into*, so it is never scanned and
therefore never cleaned. Matching a directory prunes the whole subtree.

Patterns come from two places, and they combine:

```toml
# ~/.config/rustsweep/config.toml
ignore = ["vendor", "~/Code/Rust/EMBED/ESP"]
```

```sh
rustsweep --ignore third_party --ignore '**/build-cache'
```

**`--ignore` adds to the config's list; it never replaces it.** Repeat the flag
for more patterns. This is the one setting that doesn't follow the usual
"CLI overrides config" rule, and the asymmetry is deliberate: a command line
should not be able to quietly un-protect a directory the config named.

The built-in floor is `.git`, `node_modules`, and `.jj`. You can add to it; you
cannot remove from it.

## The rules

`.gitignore`-style, with one decision point: does the pattern contain a `/` or a
glob metacharacter (`*`, `?`, `[`, `]`, `{`, `}`)?

**No** → it's a **bare name**, matched against each directory's own name at any
depth.

**Yes** → it's a **glob**, matched against each directory's absolute path.

| Pattern | Matches |
| --- | --- |
| `vendor` | a directory named `vendor`, at any depth |
| `**/build-cache` | the same thing, written as an explicit glob |
| `~/Code/*/scratch` | `~/Code/a/scratch`, but **not** `~/Code/a/b/scratch` |
| `~/Code/**/scratch` | `scratch` at any depth under `~/Code` |
| `/opt/toolchains` | that directory and everything under it |
| `a/b` | `a/b` under any ancestor — it becomes `**/a/b` |

`*` stops at a `/` and `**` spans them, the `.gitignore` reading. A leading `~`
is expanded; `./x` and `../x` resolve against the current directory; anything
else relative is anchored as `**/…`.

An unparseable pattern warns and is dropped — the rest of the list still
protects what it names, rather than the whole list failing together.

## Why bare names are worth preferring

This is a real cost difference, not a style preference.

A bare name is compared against the directory name the walk already has in hand.
That is a string comparison against a `HashSet` — free, and exactly how the
built-ins work.

A glob has to be matched against the directory's *absolute* path, and getting
that requires resolving the path — a filesystem call per directory. rustsweep
gates this on whether any glob is configured at all, so a name-only ignore list
never touches the filesystem for matching. Configure a single glob and the whole
walk switches on that cost.

Measured on a 60,693-directory tree: **8.8s** with no globs, **13.0s** with one.
About 45% slower, for one pattern.

So: `vendor` rather than `**/vendor`, unless you actually need the path to be
part of the match.

## Matching is tried twice

`matches()` tests both the path as walked and its canonical form, because only
one of the two may resemble what you wrote:

- On macOS, `/var/…` canonicalizes to `/private/var/…`. A pattern written the
  first way would otherwise never match.
- A walk rooted at a relative `--path` produces relative paths, which no
  absolute pattern could match.

A plain absolute pattern with no glob syntax is itself canonicalized when it's
compiled, so a pattern written through a symlink still matches the real
directory the walk reports.

## The search root is exempt

Pointing `--path` directly at an ignored directory scans it anyway:

```sh
# even with ignore = ["vendor"] in the config
rustsweep --path ~/Code/vendor
```

An explicit target on the command line is an explicit request, and it beats a
standing config. The patterns still apply to everything *beneath* that root.

## What you don't need to ignore

- **`target`** is not in the built-in list, and shouldn't be in yours. Build
  directories are recognized by their `CACHEDIR.TAG`, not their name — which is
  the whole point, since a build directory may have been renamed. Ignoring
  `target` by name would just hide the common case from the scan.
- **Build directories generally.** The walk never descends into any directory
  holding a `CACHEDIR.TAG`; it records it and stops. You don't pay to walk the
  inside of one, ignore list or not.
