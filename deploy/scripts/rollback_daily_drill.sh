#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-daily-rollback}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/rollback-daily-drill-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/rollback-daily-drill-${STAMP}.json"
export RUST_MIN_STACK="${RUST_MIN_STACK:-33554432}"

mkdir -p "${RUNTIME_DIR}"
RESULTS_JSON=""

run_step() {
  local name="$1"
  shift
  echo "" >> "${LOG_PATH}"
  echo "==== RUN: [${name}] $* ====" >> "${LOG_PATH}"
  (
    cd "${REPO_ROOT}"
    "$@"
  ) >> "${LOG_PATH}" 2>&1

  local esc_cmd
  esc_cmd=$(printf '%s' "$*" | sed 's/"/\\"/g')
  RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true,\"exit_code\":0},"
}

run_step "high-risk-unauthorized-rejected" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test permission_mode_runtime_enforced_e2e
run_step "canary-path-write-blocked-with-traceable-deny" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib tests::production_write_gate_blocks_canary_path_9c
run_step "canary-fail-auto-rollback-e2e" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pevo_r10_promote_canary_fail_rollback_e2e
run_step "rollout-drill-shadow-canary-full-rollback" \
  bash ./deploy/scripts/d14_rollout.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}"

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "manifest": "${MANIFEST_PATH}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "all_passed": true,
  "checks": [
    "high-risk unauthorized deny",
    "canary write gate deny with traceable reason",
    "auto rollback on canary fail",
    "daily rollout/rollback drill"
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "ROLLBACK_DAILY_DRILL_OK log=${LOG_PATH}"
echo "ROLLBACK_DAILY_DRILL_JSON=${JSON_PATH}"

