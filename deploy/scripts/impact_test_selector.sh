#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CHANGED_FILES_PATH="${1:-}"
OUTPUT_PATH="${2:-}"

args=(
  -NoProfile
  -ExecutionPolicy Bypass
  -File "${SCRIPT_DIR}/impact_test_selector.ps1"
  -RepoRoot "${REPO_ROOT}"
)

if [[ -n "${CHANGED_FILES_PATH}" ]]; then
  args+=(-ChangedFilesPath "${CHANGED_FILES_PATH}")
fi
if [[ -n "${OUTPUT_PATH}" ]]; then
  args+=(-OutputPath "${OUTPUT_PATH}")
fi

powershell "${args[@]}"
