#!/usr/bin/env bash
set -euo pipefail

out=${1:?artifact directory required}
suffix=${2:?platform suffix required}
cd "$out"
sums="SHA256SUMS-$suffix.txt"
rm -f "$sums"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -- * > "$sums"
  sha256sum -c "$sums"
else
  shasum -a 256 -- * > "$sums"
  shasum -a 256 -c "$sums"
fi
