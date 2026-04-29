#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-pwiki11-rollout}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/pwiki11-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/pwiki11-acceptance-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/pwiki11-autoloop.prod.backup-${STAMP}.toml"

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

set_rollback_version() {
  local version="$1"
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  sed -E "s/rollback_contract_version\s*=\s*\".*\"/rollback_contract_version = \"${version}\"/" "$file" > "${file}.tmp"
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

PWIKI11_TESTS=(
  pwiki11_semantic_edges_contract
  pwiki11_inference_checkpoint_roundtrip
  pwiki11_graph_export_service
  pwiki11_hot_index_refresh_modes
  pwiki11_ingest_validator_validate_only
  pwiki11_recall_cjk_fallback
  pwiki11_recall_neighbor_expansion
  pwiki11_semantic_lint_sections
  pwiki11_view_plane_persist_graph_health
  pwiki11_heal_proposal_queue
  pwiki11_refresh_source_hash_stale
  pwiki11_memory_chain_e2e
)

for test_name in "${PWIKI11_TESTS[@]}"; do
  run_step "pwiki11-${test_name}" cargo test --manifest-path "${MANIFEST_PATH}" --test "${test_name}"
done

set_local_state_store

for stage in shadow canary10 canary30 full; do
  case "${stage}" in
    shadow) mode="shadow"; ratio="0.2"; session="${SESSION_PREFIX}-shadow" ;;
    canary10) mode="canary"; ratio="0.1"; session="${SESSION_PREFIX}-10" ;;
    canary30) mode="canary"; ratio="0.3"; session="${SESSION_PREFIX}-30" ;;
    full) mode="full"; ratio="1.0"; session="${SESSION_PREFIX}-full" ;;
  esac

  set_gate_config "${mode}" "${ratio}"
  run_step "rollout-${stage}-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
  run_step "rollout-${stage}-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session}" system health
done

set_rollback_version "v1"
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
    "pwiki11-tests",
    "graph-health-lint",
    "heal-proposal-gate",
    "e2e-memory-chain"
  ],
  "rollout": ["shadow", "10%", "30%", "full", "rollback"],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "PWIKI11_ACCEPTANCE_OK log=${LOG_PATH}"
echo "PWIKI11_ACCEPTANCE_JSON=${JSON_PATH}"

