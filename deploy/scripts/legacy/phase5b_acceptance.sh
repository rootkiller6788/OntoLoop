#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/phase5b-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/phase5b-acceptance-${STAMP}.json"
mkdir -p "${RUNTIME_DIR}"

RESULTS_JSON=""

run_step() {
  local name="$1"
  shift
  local cmd=("$@")
  local printable
  printable=$(printf '%q ' "${cmd[@]}")
  echo "" >> "${LOG_PATH}"
  echo "==== RUN: [${name}] ${printable}====" >> "${LOG_PATH}"
  if (cd "${REPO_ROOT}" && "${cmd[@]}") >> "${LOG_PATH}" 2>&1; then
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${printable}\",\"passed\":true,\"exit_code\":0},"
  else
    local exit_code=$?
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${printable}\",\"passed\":false,\"exit_code\":${exit_code}},"
    exit ${exit_code}
  fi
}

run_step "cargo-check" cargo check --manifest-path "${MANIFEST_PATH}"
run_step "evo-shadow-cycle" cargo test --manifest-path "${MANIFEST_PATH}" --lib -j 1 shadow_cycle_builds_full_pipeline_outputs
run_step "query-evo-explain-counterfactual" cargo test --manifest-path "${MANIFEST_PATH}" --lib -j 1 query_plane_surfaces_evolution_decision_path_and_reject_reason
run_step "d14-rollout-chain" bash ./deploy/scripts/d14_rollout.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "phase5b-rollout" "false"

RESULTS_JSON="[${RESULTS_JSON%,}]"
cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "all_passed": true,
  "checks": [
    "counterfactual_replay_visible",
    "org_change_proposal_visible",
    "shadow_10_30_full_rollback_automated"
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "PHASE5B_ACCEPTANCE_OK log=${LOG_PATH}"
echo "PHASE5B_ACCEPTANCE_JSON=${JSON_PATH}"
