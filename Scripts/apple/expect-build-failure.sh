#!/usr/bin/env bash
# EXPERIMENTAL (T11B readiness). Builds the packaging probe and succeeds
# only if that build FAILS, and fails for a reason that names the packaged
# library. Used to show that a missing or damaged artifact is noticed and
# is not hidden by build products left from an earlier build.
#
#   Scripts/apple/expect-build-failure.sh <label> <derived data directory>
set -uo pipefail

label="$1"
derived="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"
log="$(mktemp)"

cd "${root}/Apple/PackagingProbe"
xcodebuild build-for-testing \
  -scheme LocsimPackagingProbe \
  -destination "generic/platform=iOS Simulator" \
  -derivedDataPath "${derived}" > "${log}" 2>&1
status=$?

echo "== ${label}: xcodebuild exit status ${status}"
echo "-- lines with an error, and those naming the library"
grep -nE "error:|LocsimFFI|locsim_ffi|\*\* " "${log}" | cut -c1-400 | head -40
echo "-- last 15 lines"
tail -15 "${log}" | cut -c1-400

if [ "${status}" -eq 0 ]; then
  echo "RESULT ${label}: THE BUILD SUCCEEDED. The artifact was not needed, or stale products were used."
  exit 1
fi
if ! grep -qE "LocsimFFI|locsim_ffi" "${log}"; then
  echo "RESULT ${label}: the build failed, but nothing in its log names the library. Not accepted as evidence."
  exit 1
fi
echo "RESULT ${label}: the build failed and its log names the library."
