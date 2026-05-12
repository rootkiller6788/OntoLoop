#!/usr/bin/env bash
set -euo pipefail

RUNTIME_DIR_ARG="${1:-deploy/runtime}"
DAILY_RELEASE_PACKAGE_JSON="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ "${RUNTIME_DIR_ARG}" = /* || "${RUNTIME_DIR_ARG}" =~ ^[A-Za-z]:\\ ]]; then
  RUNTIME_DIR="${RUNTIME_DIR_ARG}"
else
  RUNTIME_DIR="${REPO_ROOT}/${RUNTIME_DIR_ARG}"
fi
mkdir -p "${RUNTIME_DIR}"

if [[ -z "${DAILY_RELEASE_PACKAGE_JSON}" ]]; then
  DAILY_RELEASE_PACKAGE_JSON="${RUNTIME_DIR}/daily_release_package.json"
fi

RUNTIME_DIR_WIN="$(cygpath -w "${RUNTIME_DIR}")"
DAILY_RELEASE_PACKAGE_JSON_WIN="$(cygpath -w "${DAILY_RELEASE_PACKAGE_JSON}")"

powershell -ExecutionPolicy Bypass -File "./deploy/scripts/release_gate_report.ps1" \
  -RuntimeDir "${RUNTIME_DIR_WIN}" \
  -DailyReleasePackageJson "${DAILY_RELEASE_PACKAGE_JSON_WIN}"
