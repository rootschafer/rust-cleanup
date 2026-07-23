#!/usr/bin/env bash
# Smoke-drives the real rustsweep binary against throwaway fixture trees:
# dry-run reporting, the interactive y/n prompt, --yes, --orphans, and the
# CACHEDIR.TAG safety rule. Exits 0 iff every check passes. Never touches
# anything outside its own mktemp dir, and pins RUSTSWEEP_CONFIG so the
# user's real config can't perturb the run.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$repo"
cargo build --quiet
BIN="$repo/target/debug/rustsweep"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export RUSTSWEEP_CONFIG="$work/no-such-config.toml" # hermetic: ignore any real user config

note() { printf '\n== %s\n' "$*"; }
die() {
	echo "SMOKE FAIL: $*" >&2
	exit 1
}

# The tag cargo writes into every build dir; its body containing "cargo" is
# what makes rustsweep treat a directory as deletable at all.
cargo_tag='Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by cargo.'

make_crate() { # <dir> <name>
	mkdir -p "$1/src"
	printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$2" >"$1/Cargo.toml"
	: >"$1/src/lib.rs"
}
make_build_dir() { # <dir> — fabricated cargo build dir with a ~2MiB artifact
	mkdir -p "$1/debug"
	printf '%s\n' "$cargo_tag" >"$1/CACHEDIR.TAG"
	head -c 2097152 /dev/zero >"$1/debug/blob"
}

note "binary answers --version"
"$BIN" --version | grep -q rustsweep || die "--version"

note "dry run reports sizes and deletes nothing"
fx="$work/dry"
make_crate "$fx/a" a
make_build_dir "$fx/a/target"
out=$("$BIN" --path "$fx" --dry-run --show-size)
grep -q "Would clean" <<<"$out" || die "expected 'Would clean' in: $out"
grep -q "Would free" <<<"$out" || die "expected a 'Would free' total in: $out"
grep -q "MiB" <<<"$out" || die "expected a MiB size in: $out"
[ -d "$fx/a/target" ] || die "dry run must not delete"

note "interactive prompt: 'n' keeps, 'y' cleans"
fx="$work/prompt"
make_crate "$fx/a" a
make_build_dir "$fx/a/target"
out=$(printf 'n\n' | "$BIN" --path "$fx")
grep -q "Skipped" <<<"$out" || die "declining should be listed under Skipped, got: $out"
[ -d "$fx/a/target" ] || die "'n' must keep the target"
printf 'y\n' | "$BIN" --path "$fx" >/dev/null
[ ! -d "$fx/a/target" ] || die "'y' should clean the target"

note "--yes --orphans removes a loose build dir but never a non-cargo cache"
fx="$work/orphans"
make_build_dir "$fx/loose"
mkdir -p "$fx/other"
printf 'Signature: 8a477f597d28d172789f06886806bc55\n# created by some other tool\n' >"$fx/other/CACHEDIR.TAG"
out=$("$BIN" --path "$fx" --orphans --yes)
[ ! -d "$fx/loose" ] || die "orphan should be removed with --orphans --yes"
[ -f "$fx/other/CACHEDIR.TAG" ] || die "a non-cargo CACHEDIR.TAG dir must never be touched"

note "filters: --keep-size protects a small build dir"
fx="$work/filters"
make_crate "$fx/small" small
make_build_dir "$fx/small/target"
out=$("$BIN" --path "$fx" --keep-size 10MiB --yes)
[ -d "$fx/small/target" ] || die "a ~2MiB target must survive --keep-size 10MiB"
grep -q "kept by" <<<"$out" || die "expected a kept-by-filter line in: $out"

printf '\nSMOKE OK\n'
