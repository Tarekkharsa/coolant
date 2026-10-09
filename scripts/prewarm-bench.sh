#!/bin/sh
# Measures what `coolant prewarm` saves: the first `cargo build` in a fresh
# worktree of this repository, cold vs prewarmed from the main checkout.
#
#   scripts/prewarm-bench.sh [cargo build args]   (default: -p coolant-cli)
#
# The main checkout must have been built with the same args first. Worktrees
# go under ~/coolant-bench.noindex so Spotlight doesn't add noise.
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
ARGS="${*:--p coolant-cli}"
COOLANT="$REPO/target/debug/coolant"
BASE="$HOME/coolant-bench.noindex/prewarm-$$"
mkdir -p "$BASE"
cleanup() {
  for wt in cold warm; do git -C "$REPO" worktree remove --force "$BASE/$wt" 2>/dev/null || true; done
  git -C "$REPO" branch -D "prewarm-bench-cold-$$" "prewarm-bench-warm-$$" >/dev/null 2>&1 || true
  rm -rf "$BASE"
}
trap cleanup EXIT INT TERM

( cd "$REPO" && cargo build -q $ARGS )

measure() { # label dir
  # /usr/bin/time -p prints real/user/sys seconds on its own stderr; cargo's
  # output goes to a log so the two don't mix.
  ( cd "$2" && /usr/bin/time -p sh -c "cargo build -q $ARGS >'$BASE/$1.log' 2>&1" ) 2>"$BASE/$1.time"
  real=$(awk '/^real/ {print $2}' "$BASE/$1.time")
  cpu=$(awk '/^(user|sys)/ {s += $2} END {print s}' "$BASE/$1.time")
  printf '%-10s wall %7.1fs   cpu %7.1fs\n' "$1" "$real" "$cpu"
}

git -C "$REPO" worktree add -q "$BASE/cold" -b "prewarm-bench-cold-$$"
measure cold "$BASE/cold"

git -C "$REPO" worktree add -q "$BASE/warm" -b "prewarm-bench-warm-$$"
start=$(date +%s)
"$COOLANT" prewarm "$BASE/warm" >/dev/null
echo "prewarm    took $(( $(date +%s) - start ))s"
measure prewarmed "$BASE/warm"
