#!/usr/bin/env bash
# Compare the live test inventory against tests.lock.$(platform).
#
# Shared by verify.sh and test-verify-lock.sh so the regression cannot drift
# from the gate it claims to protect.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

platform() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) echo windows ;;
    Darwin) echo macos ;;
    *) echo linux ;;
  esac
}

# Splitting a file under the line cap dropped a whole `#[cfg(test)]` module
# twice in one day. The suite stayed green with less of it running, and the
# count did not move because two tests were added in the same change. Names are
# the only thing a comparison can be trusted on.
#
# Each OS has its own lock file. A single shared lock either skipped the check
# on two CI platforms (so unix-only tests could vanish unnoticed) or failed
# every foreign platform for #[cfg]-gated names. Per-platform locks keep the
# gate on everywhere. Refresh with `scripts/test-inventory.sh`, or the
# `test-inventory` workflow when you need another OS's file.
lock="tests.lock.$(platform)"
[ -f "$lock" ] || {
  echo "invalid tests.lock: missing $lock"
  echo "Run: scripts/test-inventory.sh"
  echo "Or dispatch the test-inventory workflow for this OS."
  exit 1
}
current=$(mktemp)
mine=""
theirs=""
trap 'rm -f "$current" ${mine:+"$mine"} ${theirs:+"$theirs"}' EXIT
scripts/test-inventory.sh "$current" > /dev/null
# Both sides through the same filter before comparing: drop the platform
# header, which is not a test name and does not sort with them, and strip any
# carriage returns, because a checkout with autocrlf on gives every line a
# trailing \r and every single test then reads as both missing and new.
#
# `comm` warns on stderr when its input is not sorted and still produces
# output, so the first version of this printed 356 spurious differences behind
# a warning nobody read. Sorting both sides here means there is nothing to warn
# about.
strip() { tail -n +2 "$1" | tr -d '\r' | LC_ALL=C sort; }
mine=$(mktemp); theirs=$(mktemp)
strip "$lock" > "$mine"
strip "$current" > "$theirs"
gone=$(LC_ALL=C comm -23 "$mine" "$theirs")
added=$(LC_ALL=C comm -13 "$mine" "$theirs")
if [ -n "$gone" ]; then
  echo "  these tests are in $lock and no longer run:"
  printf '%s\n' "$gone" | head -20 | sed 's/^/    /'
  echo "  If you deleted them on purpose, run scripts/test-inventory.sh and say why in the commit."
  exit 1
fi
if [ -n "$added" ]; then
  echo "  new tests are not in $lock yet. Run: scripts/test-inventory.sh"
  printf '%s\n' "$added" | head -10 | sed 's/^/    /'
  exit 1
fi
echo "no test has gone missing OK ($(($(wc -l < "$lock") - 1)) recorded)"
