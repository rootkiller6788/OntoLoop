#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
CONTROL_CONFIG_PATH="${2:-deploy/config/autoloop.opencode_like.toml}"
EXPERIMENT_CONFIG_PATH="${3:-deploy/config/autoloop.baseline_v0.toml}"
SPLIT="${4:-all}"
LIMIT="${5:-0}"
CASE_TIMEOUT_MS="${6:-60000}"
RETRY_COUNT="${7:-0}"
SESSION_PREFIX="${8:-benchmark-v1-compare}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PS1_PATH_WIN="$(cygpath -w "${REPO_ROOT}/deploy/scripts/benchmark_v1_compare.ps1")"
MANIFEST_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${MANIFEST_PATH}")"
CONTROL_CONFIG_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${CONTROL_CONFIG_PATH}")"
EXPERIMENT_CONFIG_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${EXPERIMENT_CONFIG_PATH}")"

powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" \
  -ManifestPath "${MANIFEST_PATH_WIN}" \
  -ControlConfigPath "${CONTROL_CONFIG_PATH_WIN}" \
  -ExperimentConfigPath "${EXPERIMENT_CONFIG_PATH_WIN}" \
  -Split "${SPLIT}" \
  -Limit "${LIMIT}" \
  -CaseTimeoutMs "${CASE_TIMEOUT_MS}" \
  -RetryCount "${RETRY_COUNT}" \
  -SessionPrefix "${SESSION_PREFIX}"
