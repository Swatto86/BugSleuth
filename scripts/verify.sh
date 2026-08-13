#!/usr/bin/env bash
# The full gate, for Linux and macOS.
#
# **The** gate — there is no second implementation. `verify.ps1` is a shim that
# runs this file through Git for Windows' Bash, because keeping a parallel
# PowerShell gate in step by hand failed three times in one day: the two
# counted lines differently, checked different file types, and built different
# things, and the third of those published a release on one platform only.
#
# Cheapest checks first, so a formatting slip fails in seconds rather than after
# a release build. The helper checks are inlined: they are a dozen lines each,
# and a separate script per check is how the duplication started.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

package=0
[ "${1:-}" = "--package" ] && package=1

say() { printf '\n== %s ==\n' "$1"; }

say "pre-push hook"
bash "$root/scripts/install-hooks.sh"

say "rust fmt"
cargo fmt --all -- --check

say "clippy"
cargo clippy --workspace --all-targets -- -D warnings

say "rust tests"
cargo test --workspace

say "frontend types"
npm run --silent build

say "frontend formatting"
# Scoped to the TypeScript the formatter has actually been applied to. The
# markup, CSS and prose are hand-laid-out and prettier's opinion of them is
# not worth the churn — reflowing the markdown docs is a large diff nobody
# asked for.
#
# The version is pinned exactly in package.json, not a caret range: two
# machines on different prettier minors format differently, and then each
# person's commit reformats the other's file.
#
# `endOfLine: "auto"` in `.prettierrc.json`, because line endings are git's job
# and prettier's default disagrees. This check passed on the machine it was
# written on and failed every Windows CI run: a fresh checkout there is CRLF,
# while the author's working tree was LF, so prettier called all 21 files
# misformatted. Formatting is still enforced — only the line ending is left to
# `core.autocrlf`, which is the thing that actually decides it.
#
# This check was vacuous when first written. `.prettierignore` held `*`
# followed by a re-inclusion, which gitignore semantics do not allow under an
# excluded parent, so it scanned nothing and passed — over a file deliberately
# misformatted to test it. Reintroduce that and this must fail.
npx --no-install prettier --check "ui/src/**/*.ts"

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

say "version agreement"
# The same version is written in three files. A release where they disagree
# publishes artifacts labelled one thing by the installer and another by the
# binary, and the tag matches whichever was remembered last.
cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
node scripts/check-version-agreement.mjs "$cargo_version"
echo "version agreement OK ($cargo_version)"

say "script permissions"
# A shell script committed from Windows arrives without its executable bit, and
# nothing on Windows ever notices. The workflows call this file as
# ./scripts/verify.sh, so both non-Windows runners failed with "permission
# denied" until the bit was set in the index — a full CI round-trip to learn
# something git knew locally.
notexec=0
while IFS= read -r entry; do
  mode=${entry%% *}
  file=${entry#*$'\t'}
  if [ "$mode" != "100755" ]; then
    echo "  not executable in git: $file (git update-index --chmod=+x '$file')"
    notexec=$((notexec + 1))
  fi
done < <(git ls-files -s -- '*.sh')
[ "$notexec" -eq 0 ] || exit 1
echo "script permissions OK"

say "no test has gone missing"
bash "$root/scripts/check-test-inventory.sh"

say "test inventory regression"
./scripts/test-verify-lock.sh
bash "$root/scripts/test-inventory-status.sh"

say "file sizes"
# 400 lines is the hard cap. Generated, vendored and lock files are exempt.
hard=400
over=0
while IFS= read -r file; do
  case "$file" in
    ./target/*|./node_modules/*|./ui/dist/*|./.git/*) continue ;;
  esac
  # Records, not newline bytes. `wc -l` reported a 401-line file with no
  # trailing newline as 400 and let it through the hard cap; awk's NR counts the
  # unterminated final record too, and is POSIX on Git Bash, Linux and macOS.
  lines=$(awk 'END { print NR }' "$file")
  if [ "$lines" -gt "$hard" ]; then
    echo "  OVER HARD CAP ($hard): $file — $lines lines"
    over=$((over + 1))
  fi
# Rust and TypeScript sources. This list lives in exactly one place now: when
# there were two gates it appeared twice, they disagreed, and the first
# cross-platform run failed on a file the other side had never checked.
done < <(find . -type f \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' \))
if [ "$over" -gt 0 ]; then
  echo ""
  echo "Split these by responsibility before committing."
  exit 1
fi
echo "file sizes OK (none over $hard lines)"

say "release build (libraries and CLI)"
# Everything EXCEPT the app crate: building bugsleuth-app with plain cargo
# poisons Tauri's dev-vs-production cache.
cargo build --release --workspace --exclude bugsleuth-app

if [ "$package" = "1" ]; then
  say "packaged build (clean)"
  # A full clean first, and not out of caution. Tauri's build script records the
  # dev-vs-production choice and cargo caches it, so once a plain release build
  # has recorded "dev", every later `tauri build` reuses it and silently makes
  # an app whose window is blank without a dev server.
  cargo clean --release
  npx --no-install tauri build
fi

printf '\nALL CHECKS PASSED\n'
