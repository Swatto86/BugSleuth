#!/usr/bin/env bash
# The project gate. Run this before pushing.
#
# Builds, runs the tests, and reports whether the tree is good.

# `pipefail` is why this is bash, not sh: `cargo test ... | tail -5` otherwise
# takes its exit status from `tail`, which succeeds even when the tests fail —
# so the gate would print "checks passed" over a red suite.
set -euo pipefail

echo "== build =="
cargo build --quiet

echo "== tests =="
cargo test --quiet 2>&1 | tail -5

echo ""
echo "checks passed"
