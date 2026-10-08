#!/usr/bin/env bash
# Runs the whole test suite. Extra arguments are passed to cargo test.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --workspace -q "$@"
