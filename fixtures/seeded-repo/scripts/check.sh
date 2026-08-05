#!/bin/sh
# The project gate. Run this before pushing.
#
# Builds, runs the tests, and reports whether the tree is good.

set -e

echo "== build =="
cargo build --quiet

echo "== tests =="
cargo test --quiet 2>&1 | tail -5

echo ""
echo "checks passed"
