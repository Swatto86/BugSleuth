#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
artifacts="$scratch/artifacts"
fake_bin="$scratch/bin"
mkdir -p "$artifacts" "$fake_bin"
printf 'release artifact\n' > "$artifacts/artifact.bin"

cat > "$fake_bin/sha256sum" <<'FAKE'
#!/usr/bin/env bash
if [ "${1:-}" = "-c" ]; then
  exit 9
fi
printf 'fake  artifact.bin\n'
FAKE
chmod +x "$fake_bin/sha256sum"

set +e
PATH="$fake_bin:$PATH" bash "$root/scripts/release-checksums.sh" \
  "$artifacts" test >/dev/null 2>&1
status=$?
set -e
[ "$status" -eq 9 ] || {
  echo "release-checksums.sh swallowed verifier exit 9 (got $status)"
  exit 1
}

bash "$root/scripts/release-checksums.sh" "$artifacts" test >/dev/null
echo "release checksum propagation OK"
