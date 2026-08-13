#!/usr/bin/env bash
# Prove a partial test listing cannot be accepted when its runner fails.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/bin"

cat > "$scratch/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo 'agreement_counts_distinct_vendors: test'
for i in {1..5000}; do printf 'stub_%s: test\n' "$i"; done
[ "${FAIL_TOOL:-}" != cargo ] || { echo 'cargo listing failed' >&2; exit 7; }
EOF

cat > "$scratch/bin/npm" <<'EOF'
#!/usr/bin/env bash
echo '✔ the scans find things at all, so a passing run means something (1ms)'
for i in {1..5000}; do printf '✔ stub %s (1ms)\n' "$i"; done
[ "${FAIL_TOOL:-}" != npm ] || { echo 'npm listing failed' >&2; exit 8; }
EOF
chmod +x "$scratch/bin/cargo" "$scratch/bin/npm"

for tool in cargo npm; do
  inventory="$scratch/$tool.lock"
  if PATH="$scratch/bin:$PATH" FAIL_TOOL="$tool" \
    bash "$root/scripts/test-inventory.sh" "$inventory" > "$scratch/$tool.log" 2>&1
  then
    echo "FAIL: test-inventory.sh accepted a failing $tool runner"
    exit 1
  fi
  [ ! -e "$inventory" ] || {
    echo "FAIL: test-inventory.sh wrote an inventory after $tool failed"
    exit 1
  }
done

inventory="$scratch/success.lock"
if ! PATH="$scratch/bin:$PATH" \
  bash "$root/scripts/test-inventory.sh" "$inventory" > "$scratch/success.log" 2>&1
then
  cat "$scratch/success.log"
  echo "FAIL: known-present checks rejected complete large runner output"
  exit 1
fi

echo "test-inventory-status OK (runner failures stop generation; large output passes)"
