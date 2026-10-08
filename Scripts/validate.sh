#!/usr/bin/env bash
# Pre-commit gate: lint + full tests.
set -euo pipefail
cd "$(dirname "$0")"
./lint.sh
./test.sh
