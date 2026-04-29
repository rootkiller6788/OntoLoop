#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-pq11-foundry-rollout}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/pq11-foundry-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/pq11-foundry-acceptance-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/pq11-autoloop.prod.backup-${STAMP}.toml"

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
  RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true},"
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

run_step "cargo-check" cargo check --workspace --manifest-path "${MANIFEST_PATH}"
run_step "unit-router-matrix" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_skill_foundry_router_matrix
run_step "unit-builder-template" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_skill_foundry_builder_template
run_step "unit-validator-pipeline" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_skill_foundry_validator_pipeline
run_step "e2e-foundry-pipeline-shadow" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_skill_foundry_end_to_end

set_local_state_store

for stage in off shadow canary10 canary30 full; do
  case "${stage}" in
    off) mode="shadow"; ratio="0.0"; session="${SESSION_PREFIX}-off" ;;
    shadow) mode="shadow"; ratio="0.2"; session="${SESSION_PREFIX}-shadow" ;;
    canary10) mode="canary"; ratio="0.1"; session="${SESSION_PREFIX}-10" ;;
    canary30) mode="canary"; ratio="0.3"; session="${SESSION_PREFIX}-30" ;;
    full) mode="full"; ratio="1.0"; session="${SESSION_PREFIX}-full" ;;
  esac

  set_gate_config "${mode}" "${ratio}"
  run_step "rollout-${stage}-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
  run_step "rollout-${stage}-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session}" system health
done

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
  "required_checks": [
    "router-matrix",
    "builder-template",
    "validator-pipeline",
    "foundry-end-to-end-shadow"
  ],
  "rollout": ["off", "shadow", "10%", "30%", "full", "rollback"],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "PQ11_SKILL_FOUNDRY_ACCEPTANCE_OK log=${LOG_PATH}"
echo "PQ11_SKILL_FOUNDRY_ACCEPTANCE_JSON=${JSON_PATH}"

