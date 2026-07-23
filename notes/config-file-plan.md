# Plan / handoff: optional config file (`~/.config/rustsweep/config.toml`)

**Superseded in part (2026-07-22, same day):** two decisions below were reversed
shortly after landing. (1) `ignore_paths` + `ignore_names` were replaced by a
single `ignore` key taking `.gitignore`-style patterns (`src/ignore.rs`); bare
names still match by name at any depth, everything else is a glob on the full
path. (2) `yes` is no longer a config key at all — auto-clean is command-line
only, so the "auto-cleaning came from config" notice is gone too. README and
`config.example.toml` are the current reference.

**Status:** DONE (2026-07-22) — implemented as specified. The §8 questions were
resolved by taking every recommendation: `~` expansion is implemented (in
`util::expand_tilde`, applied to `path` and `ignore_paths`); config-set
auto-clean stays allowed but a non-dry run that auto-cleans *because of the
config* prints "Auto-cleaning without prompts (enabled by the config file)."
first; `--no-*` override flags are deferred (option A / `value_source` works, so
the OR-merge fallback that motivated them wasn't needed); a malformed config
warns and continues with defaults; `config init` remains out of scope.

**Goal:** let users set persistent defaults for the CLI flags and maintain a
**global ignore list** (paths and/or directory names never scanned or cleaned),
via an optional TOML file at `~/.config/rustsweep/config.toml`. The file is
optional; when absent, behavior is exactly as today.

---

## 1. Scope

**In scope**
- Load an optional TOML config from an XDG-style location.
- Let it provide defaults for the existing flags.
- Add a new **global ignore** capability (`ignore_paths`, `ignore_names`) wired
  into the walker, plus an optional matching `--ignore <PATH>` CLI flag that
  *appends* to the config list.
- Precedence: **CLI flag > config value > built-in default.**
- Tests + docs.

**Out of scope (note as future work)**
- A `config init` / `--write-default-config` subcommand to scaffold the file.
- `~` expansion in config paths (call out below; implement only if easy).
- Per-project (`.rustsweep.toml`) config discovery walking up the tree.

---

## 2. Current state this must integrate with

Read these before starting; the plan references them:

- `src/cli.rs` — `Cli` (`derive(Parser)`) has `path: PathBuf` (default `"."`),
  `follow_symlinks: bool` (`-L`), `max_depth: Option<usize>` (`-d`), and a
  flattened `Flags`. `Flags` (`derive(Args, Clone, Copy)`) holds `yes_cargo`,
  `yes_dioxus`, `yes_all` (`-y`), `orphans`, `dry_run` (`-n`), `verbose` (`-v`),
  `keep_days: Option<u64>`, `keep_size: Option<u64>` (parsed by `parse_size`),
  `show_size: bool` (`-s`). `run_cli()` = parse → `configure_thread_pool()` →
  `discover()` → `build_plan()` → `clean::run()`. `parse_size(&str) -> Result<u64,String>`
  lives here (currently private).
- `src/discover.rs` — `WalkOptions { follow_symlinks, max_depth }`,
  `discover(start, options)`, and the walker. Directory pruning happens in `walk()`:
  `PRUNED_DIRS: [&str;3] = [".git","node_modules",".jj"]`, checked as
  `if !name.is_some_and(|n| PRUNED_DIRS.contains(&n)) { subdirs.push(...) }`.
  A `WalkCtx` struct already threads walk state (follow_symlinks, max_depth,
  visited, scanned).
- `src/clean.rs` — `Flags` is consumed here (`should_autoclean`, filters, sizes).
- Conventions: **tabs** for indentation; internal items are `pub(crate)`;
  modules declared in `src/lib.rs` (`pub mod cli; mod clean; mod discover; mod
  plan; mod util;`). Tests are integration tests in `tests/integration.rs`
  (spawn the built binary via `env!("CARGO_BIN_EXE_rustsweep")`), plus unit
  tests inline (`parse_size` has one in `cli.rs`).

---

## 3. User-facing behavior

### 3.1 Location & discovery
Resolve in this order (first hit wins):
1. `RUSTSWEEP_CONFIG` env var (absolute path to a config file) — **primarily
   for tests**, but a nice escape hatch.
2. `$XDG_CONFIG_HOME/rustsweep/config.toml` (if `XDG_CONFIG_HOME` is set and
   absolute).
3. `$HOME/.config/rustsweep/config.toml`.
4. Windows fallback: `$USERPROFILE/.config/rustsweep/config.toml` (keeps the
   `~/.config` shape the user asked for; do **not** use `%APPDATA%`).

> **Decision — do NOT use the `dirs`/`directories` crate.** On macOS
> `dirs::config_dir()` returns `~/Library/Application Support`, which contradicts
> the requested `~/.config`. Resolve via env vars ourselves (sketch in §5.1). No
> new dir-discovery dependency.

Missing file → silently use built-in defaults (this is the common case).

### 3.2 Format & precedence
- TOML. Every key optional. Unknown keys are an error (typo protection).
- **CLI flag > config > built-in default**, per key.
  - `Option<_>` flags merge with `.or()` (`cli.max_depth.or(config.max_depth)`).
  - `bool` flags need explicit "was it passed?" detection — see §4.
  - Ignore lists are **additive** (config ∪ CLI `--ignore`), never subtractive.

### 3.3 Example config (ship as `config.example.toml` at repo root)
```toml
# ~/.config/rustsweep/config.toml — all keys optional.

# Default search dir when --path is omitted. Use an ABSOLUTE path
# (leading ~ is NOT expanded unless we add that; see plan §8).
# path = "/Users/anders/Code"

# --- walk ---
follow_symlinks = false
# max_depth = 8

# --- cleaning (a CLI flag overrides these for that run) ---
orphans   = false
show_size = true
verbose   = false

# Auto-clean without prompting. Powerful — enable deliberately.
# yes_all    = false
# yes_cargo  = false
# yes_dioxus = false

# --- filters (same meaning/polarity as the flags) ---
# keep_days = 14          # keep dirs touched within N days
# keep_size = "500MiB"    # only clean dirs at least this big

# --- global ignore (never scanned or cleaned) ---
ignore_paths = [
  "/Users/anders/Code/Rust/EMBED/ESP",   # vendored toolchains, etc.
]
ignore_names = ["vendor", "third_party"] # dir names pruned anywhere
```

---

## 4. Key design decision: detecting explicitly-set bool flags

`ArgAction::SetTrue` bools default to `false`; a plain `Cli::parse()` can't tell
"user didn't pass `--orphans`" from "user passed nothing." That breaks
precedence, because config should be able to enable a flag while the CLI leaves
it default. **Two options — implement Option A.**

- **A (recommended): `value_source`.** Build the command, get `ArgMatches`, and
  ask clap where each value came from:
  ```rust
  use clap::{CommandFactory, FromArgMatches};
  use clap::parser::ValueSource;

  let matches = Cli::command().get_matches();
  let cli = Cli::from_arg_matches(&matches).expect("clap parsed");
  // for a bool id:
  fn cli_set(m: &clap::ArgMatches, id: &str) -> bool {
      m.value_source(id) == Some(ValueSource::CommandLine)
  }
  ```
  Then `effective_orphans = if cli_set(&matches, "orphans") { cli.flags.orphans }
  else { config.orphans.unwrap_or(false) }`. This allows config→on and lets the
  CLI win in both directions (the CLI can pass `--orphans` to force on; to force
  *off* against a config default you'd need a `--no-*` variant — see note).
  - **Arg ids**: with `#[command(flatten)] flags: Flags`, the ids are the Rust
    field names (`orphans`, `show_size`, `follow_symlinks`, `yes_all`, …).
    **Verify** with a quick `dbg!(matches.value_source("orphans"))`.
  - Optional polish: add `--no-orphans`/`--no-show-size` overrides so a user can
    disable a config default for one run. Do this with clap `overrides_with` or a
    second hidden flag. Mark as nice-to-have, not required for v1.
- **B (simpler, weaker): OR-merge.** `effective = cli.flag || config.flag`. No
  `value_source`, but a config-enabled flag can't be turned off per-run. Only
  fall back to this if A proves fiddly; note the limitation in docs.

`Option`-valued flags (`max_depth`, `keep_days`, `keep_size`) don't need this —
`.or()` handles them regardless.

---

## 5. Implementation steps

### 5.0 Dependencies (`Cargo.toml`)
Add to `[dependencies]`:
```toml
serde = { version = "1", features = ["derive"] }
toml  = "0.8"
```
(`serde` is already a transitive dep via `cargo_metadata`, but declare it
directly since we now use it.) No `dirs` crate.

### 5.1 New module `src/config.rs`
Add `mod config;` to `src/lib.rs`. Contents (sketch — tabs, `pub(crate)`):

```rust
use std::{collections::HashSet, env, path::PathBuf};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
	pub(crate) path: Option<PathBuf>,
	pub(crate) follow_symlinks: Option<bool>,
	pub(crate) max_depth: Option<usize>,
	pub(crate) orphans: Option<bool>,
	pub(crate) dry_run: Option<bool>,
	pub(crate) verbose: Option<bool>,
	pub(crate) show_size: Option<bool>,
	pub(crate) yes_cargo: Option<bool>,
	pub(crate) yes_dioxus: Option<bool>,
	pub(crate) yes_all: Option<bool>,
	pub(crate) keep_days: Option<u64>,
	pub(crate) keep_size: Option<String>, // parsed via parse_size() after load
	pub(crate) ignore_paths: Vec<PathBuf>,
	pub(crate) ignore_names: Vec<String>,
}

/// Resolve the config path per §3.1. Returns None if no home/base can be found.
pub(crate) fn config_path() -> Option<PathBuf> {
	if let Some(p) = env::var_os("RUSTSWEEP_CONFIG") {
		return Some(PathBuf::from(p));
	}
	let base = env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.filter(|p| p.is_absolute())
		.or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
		.or_else(|| env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".config")))?;
	Some(base.join("rustsweep").join("config.toml"))
}

/// Load config. Missing file → Config::default(). A present-but-broken file
/// prints a warning naming the file+error and returns defaults (so a typo never
/// silently changes behavior AND never aborts the run).
pub(crate) fn load() -> Config {
	let Some(path) = config_path() else { return Config::default() };
	let text = match std::fs::read_to_string(&path) {
		Ok(t) => t,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
		Err(e) => {
			eprintln!("Warning: couldn't read {}: {e}; using defaults.", path.display());
			return Config::default();
		}
	};
	match toml::from_str::<Config>(&text) {
		Ok(cfg) => cfg,
		Err(e) => {
			eprintln!("Warning: invalid config at {}: {e}; using defaults.", path.display());
			Config::default()
		}
	}
}
```

### 5.2 Share `parse_size`
`config.keep_size` is a `String`; parse it with the existing `parse_size`.
**Move `parse_size` from `cli.rs` to `src/util.rs` and make it `pub(crate)`**
(both `cli` and `config` merge paths use it). Keep its unit test with it.

### 5.3 Merge in `run_cli()` (`src/cli.rs`)
Rework `run_cli()` to build matches, load config, and produce the resolved
inputs. Sketch:

```rust
pub fn run_cli() {
	let matches = Cli::command().get_matches();
	let cli = Cli::from_arg_matches(&matches).expect("clap validated args");
	let cfg = crate::config::load();

	configure_thread_pool();

	let resolved = resolve(&cli, &matches, cfg); // -> Resolved { path, walk, flags }

	let discovery = discover(&resolved.path, resolved.walk);
	let (workspaces, failed) = build_plan(&discovery.candidates);
	clean::run(resolved.flags, &discovery, &workspaces, &failed);
}
```

`resolve()` produces:
- `path`: `if cli_set("path") { cli.path } else { cfg.path.unwrap_or(".") }`.
- `WalkOptions` (now extended, see §5.4): `follow_symlinks` and `max_depth`
  merged; `ignore_names` = `PRUNED`∪`cfg.ignore_names`; `ignore_paths` =
  canonicalized `cfg.ignore_paths` ∪ any `--ignore` CLI values.
- `Flags`: each bool via §4 helper; `keep_days`/`keep_size` via `.or()` (config's
  `keep_size` string → `parse_size`; on error, warn + treat as unset).

> Keep `Flags` as the `Copy` bundle `clean::run` already expects. `resolve()`
> just constructs a fully-merged `Flags`, so nothing downstream changes.

### 5.4 Wire ignore into the walker (`src/discover.rs`)
- Extend `WalkOptions`:
  ```rust
  pub(crate) struct WalkOptions {
  	pub(crate) follow_symlinks: bool,
  	pub(crate) max_depth: Option<usize>,
  	pub(crate) ignore_names: HashSet<String>, // superset of PRUNED_DIRS
  	pub(crate) ignore_paths: Vec<PathBuf>,    // canonicalized; skip self+descendants
  }
  ```
  Build `ignore_names` in `resolve()` by unioning the current `PRUNED_DIRS` const
  with config names (so `PRUNED_DIRS` stays the built-in floor). `ignore_paths`
  canonicalized once at startup (drop entries that fail to canonicalize, with a
  warning — a nonexistent ignore path is harmless).
- Thread both into `WalkCtx`.
- In `walk()`, at the subdir-collection step, replace the name check and add a
  path check:
  ```rust
  if is_dir {
  	let pruned_by_name = name.is_some_and(|n| ctx.ignore_names.contains(n));
  	if !pruned_by_name && !ctx.is_ignored_path(&entry.path()) {
  		subdirs.push(entry.path());
  	}
  }
  ```
  `is_ignored_path`: returns false fast when `ignore_paths` is empty (zero cost
  for the common case). Otherwise canonicalize the candidate and check
  `canon.starts_with(ignore)` for any ignore path. **Perf note:** the extra
  canonicalize per dir only happens when `ignore_paths` is non-empty — opt-in,
  same philosophy as sizing.
- `ignore_names.contains(n)` takes `&str`; use `HashSet<String>` and
  `.contains(n)` (or store `HashSet<Box<str>>`; `HashSet<String>::contains::<str>`
  works via `Borrow`).

### 5.5 Optional `--ignore <PATH>` CLI flag
Add to `Cli` (not `Flags`): `#[arg(long = "ignore", value_name = "PATH")] pub
ignore: Vec<PathBuf>`. Appended to `ignore_paths` in `resolve()`. Repeatable
(`--ignore a --ignore b`). Document that it *adds to*, never replaces, config.

---

## 6. Testing plan

Add to `tests/integration.rs`. Use the `RUSTSWEEP_CONFIG` env var so tests
point at a temp config without touching `~/.config`. Add a helper:

```rust
fn run_with_config(root: &Path, config: &str, args: &[&str]) -> Output {
	let cfg = root.join("rc-config.toml");
	fs::write(&cfg, config).unwrap();
	Command::new(bin())
		.arg("--path").arg(root).args(args)
		.env("RUSTSWEEP_CONFIG", &cfg)
		.stdin(Stdio::null())
		.output().unwrap()
}
```

Cases:
1. **Config enables a default** — `orphans = true` in config, no `--orphans` on
   CLI → an orphan build dir is removed (with `yes_all = true` or `--yes-all`).
2. **`show_size = true`** in config → output contains a size + "Would free" on a
   `--dry-run`.
3. **CLI overrides config (Option case)** — config `max_depth = 1`, CLI
   `--max-depth 5` → a project at depth 2 IS cleaned (5 wins).
4. **`ignore_paths`** — config ignores `<root>/skip`; a project with a target
   under `skip/` is left untouched, while a sibling project is cleaned.
5. **`ignore_names`** — config `ignore_names = ["vendor"]`; a project under
   `vendor/` is untouched.
6. **`--ignore` CLI appends** — no config; `--ignore <root>/skip` protects that
   subtree.
7. **Malformed config** — garbage TOML (or an unknown key) → stderr warning, and
   a normal project still gets cleaned (defaults applied).
8. **Missing config** — `RUSTSWEEP_CONFIG` pointing at a nonexistent file →
   behaves exactly like no config.
9. **`keep_size = "1MiB"` from config** — large target cleaned, small kept
   (reuse the existing size-fixture pattern).

Unit tests (in `config.rs`): valid parse, `deny_unknown_fields` rejects a typo,
empty file → all `None`/empty. Keep the `parse_size` unit test alongside its new
home in `util.rs`.

Existing 34 tests must stay green — none set the env var, so config resolution
should be inert for them **as long as CI/dev machines don't already have a
`~/.config/rustsweep/config.toml`**. To make tests hermetic regardless, have
`run`/`run_input` set `RUSTSWEEP_CONFIG` to a path guaranteed not to exist
(e.g. `root.join("no-config.toml")`) so a stray real config never perturbs them.
**Do this — it prevents spooky test failures on the maintainer's own box.**

---

## 7. Docs

- README: a "Configuration" section — file location, precedence, the example
  from §3.3, and the ignore semantics (additive, path prefix vs name).
- `--help`: clap `after_help`/`after_long_help` one-liner pointing at the config
  path and precedence. Mention `--ignore` appends to config.
- Ship `config.example.toml` at the repo root.

---

## 8. Decisions to confirm with the maintainer (Anders)

1. **`~` expansion** in `path` / `ignore_paths`? TOML won't expand it. Cheap to
   add (replace a leading `~/` with `$HOME`). Recommend: yes, do it — it's the
   obvious papercut. If skipped, document "absolute paths only."
2. **Config-set `yes_all` / `yes_cargo` / `yes_dioxus`**: allowed but dangerous
   (auto-delete with no prompt as a persistent default). Keep allowed? Recommend
   yes, but consider printing a one-line notice on runs where auto-clean came
   from config (not the CLI), so it's never a silent surprise.
3. **`--no-*` override flags** to disable a config default per-run (§4): worth it
   now, or defer? Recommend defer unless the OR-merge fallback (option B) is used.
4. **Malformed config = warn-and-continue** (recommended) vs hard error. Confirm.
5. Ship a `config init` subcommand later? (Out of scope now.)

---

## 9. Acceptance checklist

- [ ] `serde` + `toml` added; no `dirs` crate; builds clean.
- [ ] `src/config.rs` with `Config`, `config_path()`, `load()`; `mod config;` in lib.
- [ ] `parse_size` moved to `util.rs`, `pub(crate)`, test moved with it.
- [ ] `WalkOptions`/`WalkCtx` carry `ignore_names` + `ignore_paths`; walker prunes
      by both; zero cost when ignore lists are empty.
- [ ] `run_cli()` merges CLI > config > default (bools via `value_source`,
      Options via `.or()`); `resolve()` builds `path`, `WalkOptions`, `Flags`.
- [ ] Optional `--ignore <PATH>` appends to the ignore paths.
- [ ] New integration + unit tests (§6) pass; existing 34 stay green; tests are
      hermetic against a real `~/.config/rustsweep/config.toml`.
- [ ] `cargo clippy --all-targets` clean.
- [ ] README + `config.example.toml` + `--help` updated.
- [ ] `--help`, `--dry-run`, and precedence manually spot-checked.

---

## 10. Rough effort / order

1. Deps + `config.rs` (`Config`, `config_path`, `load`) + move `parse_size`.
2. Extend `WalkOptions`/`WalkCtx` + walker prune logic.
3. `resolve()` merge + `run_cli()` rewrite + optional `--ignore`.
4. Tests (make existing tests hermetic first, then add new).
5. Docs + example file.

Small, self-contained; ~a few hours. No changes to `plan.rs`/`clean.rs` internals
beyond receiving an already-merged `Flags`.
