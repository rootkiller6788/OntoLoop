#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PS1_PATH_WIN="$(cygpath -w "${SCRIPT_DIR}/phase_d_ablation.ps1")"

MANIFEST_PATH="${MANIFEST_PATH:-./Cargo.toml}"
SPLIT="${SPLIT:-all}"
LIMIT="${LIMIT:-120}"
REPEATS="${REPEATS:-3}"
CASE_TIMEOUT_MS="${CASE_TIMEOUT_MS:-60000}"
RETRY_COUNT="${RETRY_COUNT:-0}"
SESSION_PREFIX="${SESSION_PREFIX:-phase-d}"
BENCHMARK_KEEP_LATEST="${BENCHMARK_KEEP_LATEST:-8}"

cd "${REPO_ROOT}"
powershell -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" \
  -ManifestPath "${MANIFEST_PATH}" \
  -Split "${SPLIT}" \
  -Limit "${LIMIT}" \
  -Repeats "${REPEATS}" \
  -CaseTimeoutMs "${CASE_TIMEOUT_MS}" \
  -RetryCount "${RETRY_COUNT}" \
  -SessionPrefix "${SESSION_PREFIX}" \
  -BenchmarkKeepLatest "${BENCHMARK_KEEP_LATEST}"

