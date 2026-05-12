#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
DATASET_PATH="${3:-deploy/benchmarks/d12_real_tasks_v1.json}"
SESSION_PREFIX="${4:-d13-realbiz}"
LIMIT="${5:-52}"
CASE_TIMEOUT_MS="${6:-60000}"
PROFILE="${AUTOLOOP_PROFILE:-production-e2e}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
export AUTOLOOP_PROFILE="${PROFILE}"

PS1_PATH_WIN="$(cygpath -w "${REPO_ROOT}/deploy/scripts/d13_realbiz_benchmark_acceptance.ps1")"
MANIFEST_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${MANIFEST_PATH}")"
CONFIG_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${PROD_CONFIG_PATH}")"
DATASET_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${DATASET_PATH}")"

powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" \
  -ManifestPath "${MANIFEST_PATH_WIN}" \
  -ProdConfigPath "${CONFIG_PATH_WIN}" \
  -DatasetPath "${DATASET_PATH_WIN}" \
  -SessionPrefix "${SESSION_PREFIX}" \
  -Limit "${LIMIT}" \
  -CaseTimeoutMs "${CASE_TIMEOUT_MS}"
