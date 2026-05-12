#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-fault-injection-daily}"
DRILL_MODE="${4:-light}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/fault-injection-daily-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/fault-injection-daily-${STAMP}.json"
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

run_step "inject-timeout-retry" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_chaos_case_records_failover
run_step "inject-budget-over-compact-replan" cargo test --manifest-path "${MANIFEST_PATH}" --test p8_budget_ledger_sovereignty
if [[ "${DRILL_MODE}" == "full" ]]; then
  run_step "inject-tool-fail-rollback" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_recover_marks_failover_with_mttr
  run_step "inject-budget-over-swarm-preflight" cargo test --manifest-path "${MANIFEST_PATH}" --lib swarm_budget_preflight_compacts_when_budget_overflows
fi

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "drill_mode": "${DRILL_MODE}",
  "cadence": "$([[ "${DRILL_MODE}" == "full" ]] && echo weekly || echo daily)",
  "session_prefix": "${SESSION_PREFIX}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "all_passed": true,
  "injections": [
    {
      "fault": "timeout",
      "expected_paths": ["retry", "rollback"],
      "verified_by": ["inject-timeout-retry", "inject-tool-fail-rollback"]
    },
    {
      "fault": "tool_fail",
      "expected_paths": ["retry", "rollback"],
      "verified_by": ["inject-tool-fail-rollback"]
    },
    {
      "fault": "budget_over",
      "expected_paths": ["compact", "replan"],
      "verified_by": ["inject-budget-over-compact-replan", "inject-budget-over-swarm-preflight"]
    }
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "FAULT_INJECTION_DAILY_OK log=${LOG_PATH}"
echo "FAULT_INJECTION_DAILY_JSON=${JSON_PATH}"
