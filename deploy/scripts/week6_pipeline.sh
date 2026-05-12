#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-week6-pipeline}"
PIPELINE_MODE="${AUTOLOOP_PIPELINE_MODE:-smoke}"
RUN_FULL52="${AUTOLOOP_RUN_D13_FULL:-0}"
RUN_SOAK="${AUTOLOOP_RUN_SOAK_STABILITY:-0}"
SOAK_HOURS="${AUTOLOOP_SOAK_DURATION_HOURS:-6}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

args=(
  -ExecutionPolicy Bypass
  -File "${REPO_ROOT}/deploy/scripts/week6_pipeline.ps1"
  -ManifestPath "${MANIFEST_PATH}"
  -ProdConfigPath "${PROD_CONFIG_PATH}"
  -SessionPrefix "${SESSION_PREFIX}"
  -PipelineMode "${PIPELINE_MODE}"
  -SoakDurationHours "${SOAK_HOURS}"
)

if [[ "${RUN_FULL52}" == "1" || "${RUN_FULL52}" == "true" ]]; then
  args+=(-RunFullBenchmark52)
fi
if [[ "${RUN_SOAK}" == "1" || "${RUN_SOAK}" == "true" ]]; then
  args+=(-RunSoakStability)
fi

powershell "${args[@]}"
