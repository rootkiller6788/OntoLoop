#!/usr/bin/env bash
set -euo pipefail

RUNTIME_DIR_ARG="${1:-deploy/runtime}"
WEEK6_JSON="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ "${RUNTIME_DIR_ARG}" = /* || "${RUNTIME_DIR_ARG}" =~ ^[A-Za-z]:\\ ]]; then
  RUNTIME_DIR="${RUNTIME_DIR_ARG}"
else
  RUNTIME_DIR="${REPO_ROOT}/${RUNTIME_DIR_ARG}"
fi
mkdir -p "${RUNTIME_DIR}"

if [[ -z "${WEEK6_JSON}" ]]; then
  echo "required input missing: week6" >&2
  exit 1
fi

RUNTIME_DIR_WIN="$(cygpath -w "${RUNTIME_DIR}")"
WEEK6_JSON_WIN="$(cygpath -w "${WEEK6_JSON}")"

powershell -ExecutionPolicy Bypass -File "./deploy/scripts/daily_release_package_report.ps1" \
  -RuntimeDir "${RUNTIME_DIR_WIN}" \
  -Week6Json "${WEEK6_JSON_WIN}"
