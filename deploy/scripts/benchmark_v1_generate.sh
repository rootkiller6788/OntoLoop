#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR="${1:-deploy/benchmarks}"
OVERWRITE="${2:-true}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PS1_PATH_WIN="$(cygpath -w "${REPO_ROOT}/deploy/scripts/benchmark_v1_generate.ps1")"
OUTPUT_DIR_WIN="$(cygpath -w "${REPO_ROOT}/${OUTPUT_DIR}")"

if [[ "${OVERWRITE}" == "true" ]]; then
  powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" -OutputDir "${OUTPUT_DIR_WIN}" -Overwrite
else
  powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" -OutputDir "${OUTPUT_DIR_WIN}"
fi
