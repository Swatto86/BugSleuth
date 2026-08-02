#!/usr/bin/env bash
# Write the name of every test in the repository to tests.lock.
#
# **A test that disappears is silent.** Splitting a file under the line cap,
# this repository lost two tests twice in one day: an extraction assumed two
# `#[cfg(test)]` modules where three existed, and the third was dropped. Nothing
# failed. The suite went green with less of it running, which is the same shape
# as most of the defects here — a scan matched less than existed and returned a
# smaller answer instead of an error.
#
# The count alone would not have caught it either: two tests vanished and two
# were added in the same change. Names are what a comparison can be trusted on.
#
# Run this after adding or renaming a test; the gate compares against it and
# fails when a name it knew about is gone.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

out="${1:-tests.lock}"

# One stable name per operating system, so a version bump is not a new platform.
platform() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) echo windows ;;
    Darwin) echo macos ;;
    *) echo linux ;;
  esac
}


# Rust: every test the workspace would run, whatever crate it lives in.
rust=$(cargo test --workspace -- --list 2>/dev/null | sed -n 's/: test$//p' | sed 's/^/rust /')
# Frontend: node's own reporter, one line per test that ran. Matched on the
# leading tick rather than on the trailing duration, because the duration's
# format is the reporter's business and not something to depend on.
node=$(npm test --silent 2>&1 | grep -E '^[^ ]+ .+ \([0-9.]+ms\)$' |
  sed -E 's/^[^ ]+ (.*) \([0-9.]+ms\)$/\1/' | sed 's/^/node /')

# Neither half may be empty. The first version of this script wrote an
# inventory containing only the Rust tests, because one sed pattern quietly
# matched nothing — the exact failure this file exists to catch, in the file
# written to catch it. A half-empty lock is worse than none: it passes.
rust_count=$(printf '%s\n' "$rust" | grep -c . || true)
node_count=$(printf '%s\n' "$node" | grep -c . || true)
if [ "$rust_count" -lt 100 ] || [ "$node_count" -lt 20 ]; then
  echo "refusing to write $out: found $rust_count rust and $node_count node tests." >&2
  echo "One of the two scans matched nothing. Fix the scan, not the threshold." >&2
  exit 1
fi

{
  # Which platform this was taken on, because some tests are #[cfg(windows)]
  # and simply do not exist elsewhere. A lock taken on one platform reports
  # those as missing on another, which is a false alarm, and a check that cries
  # wolf gets switched off. The gate compares strictly on the recording
  # platform and says plainly that it is skipping elsewhere.
  # A stable token, not `uname -s` raw: Git Bash reports the Windows build
  # number in it, so a routine OS update would have looked like a different
  # platform and quietly switched the check off.
  echo "platform $(platform)"
  printf '%s\n%s\n' "$rust" "$node" | LC_ALL=C sort -u
} > "$out"
echo "$rust_count rust + $node_count node recorded in $out (on $(platform))"
