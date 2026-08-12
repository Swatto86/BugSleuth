#!/usr/bin/env bash
# Prove the test-inventory gate fails when the lock names a test that does not
# run. Drives the same script verify.sh calls — not a mirrored strip/comm copy,
# and not an absence-grep for one historical skip phrase.
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

lock="tests.lock.$(platform)"
[ -f "$lock" ] || {
  echo "FAIL: missing $lock — run scripts/test-inventory.sh first"
  exit 1
}

backup=$(mktemp)
cp "$lock" "$backup"
trap 'cp "$backup" "$lock"; rm -f "$backup"' EXIT

{
  echo "platform windows"
  echo "rust definitely_not_a_real_test_name"
} > "$lock"

set +e
out=$(bash scripts/check-test-inventory.sh 2>&1)
code=$?
set -e

if [ "$code" -eq 0 ]; then
  echo "FAIL: check-test-inventory.sh exited 0 against a planted missing test"
  printf '%s\n' "$out"
  exit 1
fi

if ! printf '%s\n' "$out" | grep -q 'definitely_not_a_real_test_name'; then
  echo "FAIL: planted missing test was not mentioned by the inventory checker"
  printf '%s\n' "$out"
  exit 1
fi

echo "test-verify-lock OK (check-test-inventory.sh failed on planted name)"
