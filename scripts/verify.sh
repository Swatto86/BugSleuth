#!/usr/bin/env bash
# The full gate, for Linux and macOS.
#
# Mirrors `verify.ps1` step for step and in the same order — cheapest first, so
# a formatting slip fails in seconds rather than after a release build. The two
# must not drift: a check that runs on one platform and not the other means
# "green" depends on who ran it, and this project has already learned that
# lesson once with two report renderers.
#
# The helper checks are inlined rather than given their own scripts. They are a
# dozen lines each, and a third and fourth copy of the same rule is exactly what
# the contract lane was taught to hunt for.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

package=0
[ "${1:-}" = "--package" ] && package=1

say() { printf '\n== %s ==\n' "$1"; }

say "rust fmt"
cargo fmt --all -- --check

say "clippy"
cargo clippy --workspace --all-targets -- -D warnings

say "rust tests"
cargo test --workspace

say "frontend types"
npm run --silent build

say "frontend assets"
# The failure this catches: a build that half-succeeds, leaving index.html
# referencing a bundle that is not on disk. The app then starts to a blank
# window and every other check still passes.
dist="ui/dist"
index="$dist/index.html"
[ -f "$index" ] || { echo "no built frontend at $index"; exit 1; }
referenced=$(grep -oE '(src|href)="[^"]+"' "$index" | sed -E 's/.*="([^"]+)".*/\1/' | grep -E '^/?assets/' || true)
[ -n "$referenced" ] || { echo "index.html references no bundled assets, which cannot be right"; exit 1; }
count=0
while IFS= read -r asset; do
  [ -n "$asset" ] || continue
  path="$dist/${asset#/}"
  [ -f "$path" ] || { echo "index.html references $asset, which is not on disk"; exit 1; }
  count=$((count + 1))
done <<< "$referenced"
echo "frontend assets OK ($count referenced, all present)"

say "frontend tests"
npm test --silent

say "file sizes"
# 400 lines is the hard cap. Generated, vendored and lock files are exempt, as
# are the fixtures, which are deliberately awful on purpose.
hard=400
over=0
while IFS= read -r file; do
  case "$file" in
    ./target/*|./node_modules/*|./ui/dist/*|./fixtures/*|./.git/*) continue ;;
  esac
  lines=$(wc -l < "$file")
  if [ "$lines" -gt "$hard" ]; then
    echo "  OVER HARD CAP ($hard): $file — $lines lines"
    over=$((over + 1))
  fi
# The same extensions check-file-size.ps1 uses. The two lists must match or the
# gates disagree about what "green" means, which is the whole failure this file
# exists to avoid — and it happened immediately: this one included CSS and the
# other did not, so the first cross-platform run failed on a file Windows had
# never been checking.
done < <(find . -type f \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' \))
if [ "$over" -gt 0 ]; then
  echo ""
  echo "Split these by responsibility before committing."
  exit 1
fi
echo "file sizes OK (none over $hard lines)"

if [ "$package" != "1" ]; then
  say "release build (libraries and CLI)"
  # Everything EXCEPT the app crate, exactly as verify.ps1 does: building
  # bugsleuth-app with plain cargo poisons Tauri's dev-vs-production cache.
  #
  # This was missing, and it was not cosmetic. The release workflow collected
  # a CLI binary that only existed because the Windows gate had built it, so
  # v0.2.0 published on Windows and failed on Linux and macOS. Two gates that
  # build different things are two different definitions of green.
  cargo build --release --workspace --exclude bugsleuth-app
fi

if [ "$package" = "1" ]; then
  say "packaged build (clean)"
  # A full clean first, and not out of caution. Tauri's build script records the
  # dev-vs-production choice and cargo caches it, so once a plain release build
  # has recorded "dev", every later `tauri build` reuses it and silently makes
  # an app whose window is blank without a dev server.
  cargo clean --release
  npx --yes @tauri-apps/cli@^2 build
fi

printf '\nALL CHECKS PASSED\n'
