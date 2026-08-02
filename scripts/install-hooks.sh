#!/usr/bin/env bash
# Install the git hooks that make a failing gate impossible to push past.
#
# Not a nicety. The gate caught `main.ts` crossing the hard line cap and the
# commit went to the remote anyway, because the command was
#
#     ./scripts/verify.sh 2>&1 | tail -3 && git commit && git push
#
# and a pipeline exits with the status of its *last* stage, so `tail`'s success
# masked the gate's failure. The check ran, said the right thing, and nothing
# consumed the answer — the same shape as most of the defects this repository
# has found in itself.
#
# A person remembering to read an exit code is not a control. A hook is.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
hook="$root/.git/hooks/pre-push"

cat > "$hook" <<'HOOK'
#!/usr/bin/env bash
# Runs the full gate before anything reaches the remote. Installed by
# scripts/install-hooks.sh; see that file for why this is not optional.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
echo "pre-push: running the full gate"
if ! "$root/scripts/verify.sh"; then
  echo ""
  echo "pre-push: the gate failed, so nothing was pushed."
  echo "Fix it, or push with --no-verify only if you have said out loud why."
  exit 1
fi
HOOK

chmod +x "$hook"
echo "installed $hook"
