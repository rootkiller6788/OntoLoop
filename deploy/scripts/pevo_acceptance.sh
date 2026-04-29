#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-pevo-rollout}"
ARTIFACT_PATH="${4:-/d/AutoLoop/autoloop-app/deploy/runtime/pevo-shadow-bill-replica.html}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/pevo-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/pevo-acceptance-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/pevo-autoloop.prod.backup-${STAMP}.toml"

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

run_artifact_shadow() {
  local session_id="$1"
  local trace_id="trace:${session_id}:artifact-shadow"
  local prompt_file="${RUNTIME_DIR}/pevo-artifact-prompt-${STAMP}.txt"
  cat > "${prompt_file}" <<EOF
你是执行代理。必须使用工具写入文件，不允许仅文本回答。
任务：复刻一个账单展示网页，输出单文件 HTML（内联 CSS，桌面和移动端可用）。

\`\`\`json
{
  "api_version": "artifact_delivery/v1",
  "requires_artifact": true,
  "target_path": "${ARTIFACT_PATH}",
  "validation_rules": {
    "exists_required": true,
    "readable_required": true,
    "expected_mime": "text/html",
    "min_size_bytes": 200
  }
}
\`\`\`
EOF
  local prompt
  prompt="$(cat "${prompt_file}")"
  prompt="${prompt//\"/\\\"}"
  run_step "artifact-shadow-run" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" --swarm --message "${prompt}"

  local proof_out="${RUNTIME_DIR}/pevo-artifact-proof-${STAMP}.raw.log"
  (
    cd "${REPO_ROOT}"
    cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" system artifact proof --artifact-path "${ARTIFACT_PATH}" --trace-id "${trace_id}"
  ) > "${proof_out}" 2>&1
  cat "${proof_out}" >> "${LOG_PATH}"

  if [[ ! -f "${ARTIFACT_PATH}" ]]; then
    echo "artifact hard acceptance failed: file missing ${ARTIFACT_PATH}" >> "${LOG_PATH}"
    exit 1
  fi
  if ! grep -q '"status": "ready"' "${proof_out}"; then
    echo "artifact hard acceptance failed: artifact proof status is not ready" >> "${LOG_PATH}"
    exit 1
  fi
  if ! grep -q '"relation_write_proofs": \[' "${proof_out}"; then
    echo "artifact hard acceptance failed: relation_write_proofs missing" >> "${LOG_PATH}"
    exit 1
  fi

  local sha
  sha="$(sha256sum "${ARTIFACT_PATH}" | awk '{print $1}')"
  local artifact_report="${RUNTIME_DIR}/pevo-artifact-proof-${STAMP}.json"
  cat > "${artifact_report}" <<EOF
{
  "session_id": "${session_id}",
  "trace_id": "${trace_id}",
  "artifact_path": "${ARTIFACT_PATH}",
  "sha256": "${sha}",
  "proof_log": "${proof_out}"
}
EOF
  RESULTS_JSON+="{\"name\":\"artifact-shadow-proof-verified\",\"command\":\"system artifact proof + sha256 verify\",\"passed\":true,\"artifact_report\":\"${artifact_report}\"},"
}

RESULTS_JSON=""

run_step "cargo-check" cargo check --workspace --manifest-path "${MANIFEST_PATH}"
run_step "evolution-shadow-cycle-core" cargo test --manifest-path "${MANIFEST_PATH}" --lib shadow_cycle_builds_full_pipeline_outputs
run_step "query-replay-evolution-explain" cargo test --manifest-path "${MANIFEST_PATH}" --lib query_plane_surfaces_evolution_decision_path_and_reject_reason
run_step "full-chain-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test p10_day10_acceptance_e2e
run_step "e2e-no-bypass-gate" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_no_bypass_gate_e2e
run_step "e2e-no-bypass-mediator" cargo test --manifest-path "${MANIFEST_PATH}" --test mediator_no_bypass_e2e
run_step "artifact-gate-write-evidence-required" cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_requires_write_evidence_even_if_file_exists
run_step "budget-preflight-and-ledger-hard-check" cargo test --manifest-path "${MANIFEST_PATH}" --test p8_budget_ledger_sovereignty

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
  if [[ "${stage}" == "shadow" ]]; then
    run_artifact_shadow "${SESSION_PREFIX}-artifact-shadow"
  fi

  if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    run_step "rollout-${stage}-workload" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session}" --swarm --message "run evolution canary workload"
  else
    echo "INFO: skip workload on stage ${stage} because OPENAI_API_KEY is empty" >> "${LOG_PATH}"
    RESULTS_JSON+="{\"name\":\"rollout-${stage}-workload\",\"command\":\"cargo run ... --swarm --message 'run evolution canary workload'\",\"passed\":true,\"skipped\":true,\"reason\":\"OPENAI_API_KEY missing\"},"
  fi
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
    "evolution-shadow-cycle-core",
    "query-replay-evolution-explain",
    "full-chain-e2e",
    "artifact-hard-gate-and-proof",
    "no-bypass-gate-mediator",
    "budget-preflight-ledger"
  ],
  "rollout": ["shadow", "10%", "30%", "full", "rollback"],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "PEVO_ACCEPTANCE_OK log=${LOG_PATH}"
echo "PEVO_ACCEPTANCE_JSON=${JSON_PATH}"

