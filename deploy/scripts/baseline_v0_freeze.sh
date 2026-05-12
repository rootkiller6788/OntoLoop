#!/usr/bin/env bash
set -euo pipefail

CONFIG_PATH="${1:-deploy/config/autoloop.baseline_v0.toml}"
RUNTIME_DIR="${2:-deploy/runtime}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

PS1_PATH_WIN="$(cygpath -w "${REPO_ROOT}/deploy/scripts/baseline_v0_freeze.ps1")"
CONFIG_PATH_WIN="$(cygpath -w "${REPO_ROOT}/${CONFIG_PATH}")"
RUNTIME_DIR_WIN="$(cygpath -w "${REPO_ROOT}/${RUNTIME_DIR}")"

powershell -NoProfile -ExecutionPolicy Bypass -File "${PS1_PATH_WIN}" \
  -ConfigPath "${CONFIG_PATH_WIN}" \
  -RuntimeDir "${RUNTIME_DIR_WIN}"
