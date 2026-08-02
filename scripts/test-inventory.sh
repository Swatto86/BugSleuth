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


# `|| true` on both, deliberately. Under `set -euo pipefail` a `grep` that
# matches nothing exits 1 and kills this script *without printing anything* —
# which is what happened on one CI runner, and cost a round-trip to learn
# nothing. An empty result is the guard's business below, and the guard says
# which half came back empty.
rust_raw=$(cargo test --workspace -- --list 2>/dev/null || true)
rust=$(printf '%s\n' "$rust_raw" | sed -n 's/: test$//p' | sed 's/^/rust /' || true)

# Node chooses its reporter by whether stdout is a terminal: `spec` for a
# person, `tap` for a pipe. So this read one format on a developer's machine
# and a different one on CI, where it matched nothing and recorded zero
# frontend tests. `package.json` now names the reporter, which is the one place
# the test command lives — passing a flag here instead would be the same
# command written down twice, and they would drift.
node_raw=$(npm test --silent 2>&1 || true)
# Matched on the trailing duration, not on the reporter's tick: the tick is
# non-ASCII and its passage through a pipe is one more thing that can differ
# between platforms for no good reason.
node=$(printf '%s\n' "$node_raw" |
  sed -n -E 's/^[^ ]+ (.*) \([0-9.]+ms\)$/\1/p' | sed 's/^/node /' || true)

# Neither half may be empty. The first version of this script wrote an
# inventory containing only the Rust tests, because one sed pattern quietly
# matched nothing — the exact failure this file exists to catch, in the file
# written to catch it. A half-empty lock is worse than none: it passes.
rust_count=$(printf '%s\n' "$rust" | grep -c . || true)
node_count=$(printf '%s\n' "$node" | grep -c . || true)
if [ "$rust_count" -lt 100 ] || [ "$node_count" -lt 20 ]; then
  echo "refusing to write $out: found $rust_count rust and $node_count node tests." >&2
  echo "One of the two scans matched nothing. Fix the scan, not the threshold." >&2
  echo "--- last 15 lines the rust scan was given ---" >&2
  printf '%s\n' "$rust_raw" | tail -15 >&2
  echo "--- last 15 lines the node scan was given ---" >&2
  printf '%s\n' "$node_raw" | tail -15 >&2
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
  # `sort`, not `sort -u`. Sixteen frontend tests share a name with a test in
  # another file, and deduplicating collapsed each pair to one line — so
  # deleting one of them would have left the lock satisfied. Duplicates are
  # kept and compared as a multiset, which is what `comm` does anyway.
  printf '%s\n%s\n' "$rust" "$node" | LC_ALL=C sort
} > "$out"
echo "$rust_count rust + $node_count node recorded in $out (on $(platform))"
