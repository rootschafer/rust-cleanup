# rust-cleanup

A simple tool to save space on your computer by cleaning up build files for rust projects. Supports regular Rust projects and Dioxus projects. 


# Install

I choose to not publish this crate on crates.io because it's not properly documented yet.

1. Clone the github repo somewhere

2. Install with:

```rust
cargo install --path .
```

# Usage:

Output of `rust-cleanup --help`:

```
Usage: rust-cleanup [OPTIONS]

Options:
  -p, --path <PATH>        Sets the starting directory for the search [default: .]
  -L, --follow-symlinks    Follow symlinked directories while searching
  -d, --max-depth <DEPTH>  Limit how many directory levels below the search root to descend
      --ignore <PATH>      Never scan (or clean) anything inside this directory (repeatable)
      --yes-cargo          Automatically clean non-Dioxus Rust projects without prompting
      --yes-dioxus         Automatically clean Dioxus projects without prompting
  -y, --yes-all            Automatically clean all projects without prompting for a yes or a no
      --orphans            Also remove Cargo build dirs that aren't inside any discovered project
  -n, --dry-run            Show what would be cleaned without deleting anything or prompting
  -v, --verbose            List the projects that `cargo metadata` could not read
  -s, --show-size          Show each build dir's size before you decide (and a freed-space total)
      --keep-days <DAYS>   Keep build dirs touched within the last N days; only clean older ones
      --keep-size <SIZE>   Only clean build dirs at least this large, e.g. 500MB, 1GiB
  -h, --help               Print help
  -V, --version            Print version
```

# Configuration

Persistent defaults can live in an **optional** config file at
`~/.config/rust-cleanup/config.toml` (or `$XDG_CONFIG_HOME/rust-cleanup/config.toml`
if that's set; `$RUST_CLEANUP_CONFIG` overrides both). With no config file,
behavior is exactly as it is without one — nothing to set up.

**Precedence is CLI flag > config value > built-in default**, per setting. So a
config can turn something on for every run, and passing the flag explicitly
still wins for that run. Every key is optional and named after its flag:

```toml
# path = "~/Code"       # default search dir when --path is omitted
# max_depth = 8
orphans   = false
show_size = true
# keep_days = 14        # keep dirs touched within the last N days
# keep_size = "500MiB"  # only clean dirs at least this big

# Auto-clean without prompting. Powerful — enable deliberately.
# yes_all = false

# --- global ignore: never scanned, so never cleaned ---
ignore_paths = ["~/Code/Rust/EMBED/ESP"]  # whole trees, by path prefix
ignore_names = ["vendor", "third_party"]  # dir names, pruned anywhere
```

See [`config.example.toml`](config.example.toml) for the annotated full set.

Notes:

- **Ignore lists are additive.** `--ignore <PATH>` *adds to* `ignore_paths`
  rather than replacing it, and `ignore_names` adds to the always-pruned
  built-ins (`.git`, `node_modules`, `.jj`). Nothing in the config can
  *un*-protect a directory.
- `ignore_paths` matches by path prefix, so a whole subtree is skipped;
  `ignore_names` matches a directory's name at any depth. A leading `~` is
  expanded in both (and in `path`).
- An unknown key is an error, so a typo can't silently do nothing. A broken
  config prints a warning naming the file and the problem, then the run
  continues with the built-in defaults.
- If auto-cleaning is enabled by the config rather than the command line, the
  run says so before deleting anything.
