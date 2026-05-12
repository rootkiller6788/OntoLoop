#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SPLIT="${3:-dev}"
LIMIT="${4:-0}"
CASE_TIMEOUT_MS="${5:-60000}"
RETRY_COUNT="${6:-0}"
SESSION_PREFIX="${7:-benchmark-v1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PS1_PATH_WIN="$(cygpath -w "${REPO_ROOT}/deploy/scripts/benchmark_v1_eval.ps1")"
MANIFEST_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${MANIFEST_PATH}")"
CONFIG_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${PROD_CONFIG_PATH}")"

powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" \
  -ManifestPath "${MANIFEST_PATH_WIN}" \
  -ProdConfigPath "${CONFIG_PATH_WIN}" \
  -Split "${SPLIT}" \
  -Limit "${LIMIT}" \
  -CaseTimeoutMs "${CASE_TIMEOUT_MS}" \
  -RetryCount "${RETRY_COUNT}" \
  -SessionPrefix "${SESSION_PREFIX}"
