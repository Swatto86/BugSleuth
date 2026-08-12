#!/usr/bin/env bash
# Prove the test-inventory gate never skips on a foreign-looking lock header.
# Before per-platform locks, a mismatched `platform …` line printed "skipped:"
# and exited 0; after the fix the gate always diffs against tests.lock.$(os).
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

if grep -q 'skipped: tests.lock was recorded' scripts/verify.sh; then
  echo "FAIL: verify.sh still skips the inventory gate on platform mismatch"
  exit 1
fi

lock="tests.lock.$(platform)"
[ -f "$lock" ] || {
  echo "FAIL: missing $lock — run scripts/test-inventory.sh first"
  exit 1
}

# Mirror the gate in verify.sh: plant a clearly fake name in this OS's lock
# (header deliberately says windows even on other OSes) and require a non-zero
# exit from the same strip/comm path. Running the full verify.sh would also
# fail here, but only after minutes of unrelated work.
backup=$(mktemp)
cp "$lock" "$backup"
trap 'cp "$backup" "$lock"; rm -f "$backup" "$current" "$mine" "$theirs"' EXIT

{
  echo "platform windows"
  echo "rust definitely_not_a_real_test_name"
} > "$lock"

current=$(mktemp)
mine=$(mktemp)
theirs=$(mktemp)
scripts/test-inventory.sh "$current" > /dev/null
strip() { tail -n +2 "$1" | tr -d '\r' | LC_ALL=C sort; }
strip "$lock" > "$mine"
strip "$current" > "$theirs"
gone=$(LC_ALL=C comm -23 "$mine" "$theirs")
added=$(LC_ALL=C comm -13 "$mine" "$theirs")

if [ -z "$gone" ] && [ -z "$added" ]; then
  echo "FAIL: planted lock compared equal to live inventory"
  exit 1
fi

if ! printf '%s\n' "$gone" | grep -q 'definitely_not_a_real_test_name'; then
  echo "FAIL: planted missing test was not detected as gone"
  printf 'gone:\n%s\nadded:\n%s\n' "$gone" "$added"
  exit 1
fi

echo "test-verify-lock OK (gate would fail on $lock; skip path removed)"
