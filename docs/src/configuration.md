# Configuration

The config file is **optional**. With no config file, rustsweep behaves exactly
as if the feature didn't exist — there is nothing to set up before the first
run.

## Where it lives

The first of these that applies wins:

1. `$RUSTSWEEP_CONFIG`, if set — the full path to the file, not a directory.
   This is an escape hatch (and what the test suite uses to stay hermetic).
2. `$XDG_CONFIG_HOME/rustsweep/config.toml`, if `$XDG_CONFIG_HOME` is set to an
   absolute path.
3. `~/.config/rustsweep/config.toml`.

## Precedence

**CLI flag > config value > built-in default**, decided per setting.

A config file can turn something on for every run, and passing the flag
explicitly on the command line still wins for that run. The one thing it can't
do is turn something *off* that the config turned on — there are no `--no-*`
flags today. Use `$RUSTSWEEP_CONFIG` pointed at an empty file if you need a
run with the config out of the way entirely.

There is one deliberate exception to "per setting": `ignore` is **additive**.
`--ignore` extends the config's list rather than replacing it, so a command line
can never quietly un-protect a directory the config named. See
[Ignore patterns](ignore-patterns.md).

## The keys

Every key is optional and named after its flag.

| Key | Type | Flag |
| --- | --- | --- |
| `path` | string (a leading `~` is expanded) | `--path` |
| `follow_symlinks` | bool | `--follow-symlinks` |
| `max_depth` | integer | `--max-depth` |
| `orphans` | bool | `--orphans` |
| `dry_run` | bool | `--dry-run` |
| `verbose` | bool | `--verbose` |
| `show_size` | bool | `--show-size` |
| `keep_days` | integer | `--keep-days` |
| `keep_size` | string, e.g. `"500MiB"` | `--keep-size` |
| `ignore` | array of strings | `--ignore` (additive) |

## <a id="there-is-no-yes-key"></a>There is no `yes` key

Cleaning without a prompt is the one thing rustsweep does that you can't undo.
Leaving that armed in a file — where it applies to every future run, including
ones you fire off without thinking about which directory you're in — is a
different risk from typing it once. So `--yes` is command-line only, and `yes`
in the config is not merely ignored but **rejected as an unknown key**:

```console
$ rustsweep --path ~/Code
Warning: invalid config at ~/.config/rustsweep/config.toml: unknown field `yes`; using defaults.
```

Note what happens there: the whole file is discarded and the run continues on
built-in defaults. Which brings us to —

## When the file is wrong

- **An unknown key is an error.** A typo like `orphan = true` can't silently do
  nothing; you get told.
- **A broken file warns and the run continues** with the built-in defaults,
  naming the file and the problem. A bad config should never change what gets
  deleted, but it also shouldn't stop you from cleaning.
- **A bad `keep_size` string** is the one narrower case: it warns, that single
  setting is treated as unset, and the rest of the config still applies.

## The annotated example

This is [`config.example.toml`](https://github.com/rootschafer/rustsweep/blob/main/config.example.toml)
from the repository, included here directly so it can't drift from the file
that ships:

```toml
{{#include ../../config.example.toml}}
```
