#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-sandbox-rollout}"
INCLUDE_BROKER_ACK="${4:-false}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/sandbox-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/sandbox-acceptance-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/sandbox-autoloop.prod.backup-${STAMP}.toml"

mkdir -p "${RUNTIME_DIR}"
cp "${REPO_ROOT}/${PROD_CONFIG_PATH}" "${BACKUP_PATH}"

restore_config() {
  if [[ -f "${BACKUP_PATH}" ]]; then
    cp "${BACKUP_PATH}" "${REPO_ROOT}/${PROD_CONFIG_PATH}"
  fi
}
trap restore_config EXIT

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
  RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true,\"optional\":false},"
}

run_optional_step() {
  local name="$1"
  shift
  echo "" >> "${LOG_PATH}"
  echo "==== RUN OPTIONAL: [${name}] $* ====" >> "${LOG_PATH}"
  set +e
  (
    cd "${REPO_ROOT}"
    "$@"
  ) >> "${LOG_PATH}" 2>&1
  local code=$?
  set -e

  local esc_cmd
  esc_cmd=$(printf '%s' "$*" | sed 's/"/\\"/g')
  if [[ $code -eq 0 ]]; then
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true,\"optional\":true},"
  else
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":false,\"optional\":true,\"exit_code\":${code}},"
  fi
}

set_gate_config() {
  local mode="$1"
  local ratio="$2"
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  sed -E "s/gate_mode\s*=\s*\".*\"/gate_mode = \"${mode}\"/" "$file" | \
    sed -E "s/gate_enforce_ratio\s*=\s*[0-9.]+/gate_enforce_ratio = ${ratio}/" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

set_local_state_store() {
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  sed -E "s/backend\s*=\s*\"sdk\"/backend = \"in_memory\"/" "$file" | \
    sed -E "s#uri\s*=\s*\"http://state_store:3000\"#uri = \"http://127.0.0.1:3000\"#" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

RESULTS_JSON=""

run_step "cargo-check" cargo check --manifest-path "${MANIFEST_PATH}"

# 1) contract_compat
run_step "contract_compat" cargo test --manifest-path "${MANIFEST_PATH}" --lib sandbox_contract_compatible

# 2) runtime_class_dispatch
run_step "runtime_class_dispatch" cargo test --manifest-path "${MANIFEST_PATH}" --lib high_risk_trust_plan_maps_to_trusted_runtime_class

# 3) high_risk_trustbridge_enforced
run_step "high_risk_trustbridge_enforced" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::trust_bridge::tests::attestation_gate_requires_env_when_enabled

# 4) hook_5phase_pipeline
run_step "hook_5phase_pipeline" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::hook_runtime::tests::legacy_stage_maps_to_phase
run_step "hook_5phase_pipeline_runtime_chain" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::

# 5) broker_delivery_ack (optional)
if [[ "${INCLUDE_BROKER_ACK}" == "true" ]]; then
  run_optional_step "broker_delivery_ack" cargo test --manifest-path "${MANIFEST_PATH}" --lib task_topics_roundtrip_and_ack
fi

set_local_state_store

set_gate_config "shadow" "0.2"
run_step "rollout-shadow-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollout-shadow-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-shadow" system health

set_gate_config "canary" "0.1"
run_step "rollout-canary10-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollout-canary10-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-10" system health

set_gate_config "canary" "0.3"
run_step "rollout-canary30-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollout-canary30-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-30" system health

set_gate_config "full" "1.0"
run_step "rollout-full-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollout-full-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-full" system health

set_gate_config "shadow" "0.2"
run_step "rollback-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollback-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-rollback" system health

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "backup_config": "${BACKUP_PATH}",
  "all_passed": true,
  "required_groups": [
    "contract_compat",
    "runtime_class_dispatch",
    "high_risk_trustbridge_enforced",
    "hook_5phase_pipeline",
    "broker_delivery_ack(optional)"
  ],
  "rollout": ["shadow", "canary(10%)", "30%", "full", "rollback"],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "SANDBOX_ACCEPTANCE_OK log=${LOG_PATH}"
echo "SANDBOX_ACCEPTANCE_JSON=${JSON_PATH}"

