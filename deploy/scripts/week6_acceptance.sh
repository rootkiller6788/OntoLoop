#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-week6-rollout}"
ARTIFACT_PATH="${4:-/d/AutoLoop/autoloop-app/deploy/runtime/week6-shadow-bill-replica.html}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/week6-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/week6-acceptance-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/week6-autoloop.prod.backup-${STAMP}.toml"
REPLAY_OUT="${RUNTIME_DIR}/week6-replay-report-${STAMP}.json"
BENCHMARK_OUT="${RUNTIME_DIR}/week6-benchmark-${STAMP}.json"
BENCHMARK_COMPARE_OUT="${RUNTIME_DIR}/week6-benchmark-compare-${STAMP}.json"
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

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
  local prompt_file="${RUNTIME_DIR}/week6-artifact-prompt-${STAMP}.txt"
  cat > "${prompt_file}" <<EOF
你是执行代理。必须使用工具写入文件，不允许仅文本回答。
任务：复刻一个账单展示网页，输出单文件 HTML（内联 CSS，桌面和移动端可用）。
完成标准：文件必须写入 target_path，且可被 artifact proof 查询到。

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

run_benchmark_shadow() {
  local session_id="$1"
  local previous_report=""
  previous_report="$(ls -1t "${RUNTIME_DIR}"/week6-benchmark-*.json 2>/dev/null | head -n 1 || true)"

  run_step "d12-real-benchmark-run" env AUTOLOOP_BENCHMARK_SHADOW_SAFE=1 AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS=4000 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" system benchmark run --limit 52 --output "${BENCHMARK_OUT}"

  if [[ ! -f "${BENCHMARK_OUT}" ]]; then
    echo "benchmark report file was not generated: ${BENCHMARK_OUT}" >> "${LOG_PATH}"
    exit 1
  fi

  python - <<PY
import json
from pathlib import Path

new_path = Path(r"${BENCHMARK_OUT}")
old_path = Path(r"${previous_report}") if "${previous_report}" else None
compare_path = Path(r"${BENCHMARK_COMPARE_OUT}")
new = json.loads(new_path.read_text(encoding="utf-8"))

payload = {
    "generated_at": "${STAMP}",
    "session_id": "${session_id}",
    "new_report": str(new_path),
    "old_report": str(old_path) if old_path and old_path.exists() else None,
    "new": {
        "total": new.get("total"),
        "passed": new.get("passed"),
        "failed": new.get("failed"),
        "success_rate": new.get("success_rate"),
        "total_retry_count": new.get("total_retry_count"),
        "average_retry_count": new.get("average_retry_count"),
        "failure_reason_distribution": new.get("failure_reason_distribution"),
        "evidence_ref": new.get("evidence_ref"),
    },
    "delta": None,
}

if old_path and old_path.exists() and old_path != new_path:
    old = json.loads(old_path.read_text(encoding="utf-8"))
    payload["delta"] = {
        "success_rate": float(new.get("success_rate", 0.0)) - float(old.get("success_rate", 0.0)),
        "total_retry_count": int(new.get("total_retry_count", 0)) - int(old.get("total_retry_count", 0)),
        "average_retry_count": float(new.get("average_retry_count", 0.0)) - float(old.get("average_retry_count", 0.0)),
        "failed": int(new.get("failed", 0)) - int(old.get("failed", 0)),
    }

compare_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
PY

  RESULTS_JSON+="{\"name\":\"d12-real-benchmark-compare\",\"command\":\"system benchmark run + compare old/new\",\"passed\":true,\"benchmark_report\":\"${BENCHMARK_OUT}\",\"benchmark_compare\":\"${BENCHMARK_COMPARE_OUT}\"},"
}
\`\`\`
EOF

  local prompt
  prompt="$(cat "${prompt_file}")"
  prompt="${prompt//\"/\\\"}"
  run_step "artifact-shadow-run" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" --swarm --message "${prompt}"

  local proof_out="${RUNTIME_DIR}/week6-artifact-proof-${STAMP}.raw.log"
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
  local artifact_report="${RUNTIME_DIR}/week6-artifact-proof-${STAMP}.json"
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
run_step "e2e-intent-execute-verify-persist-replay" cargo test --manifest-path "${MANIFEST_PATH}" --test p10_day10_acceptance_e2e
run_step "e2e-compiler-executor-verifier-closed-loop" cargo test --manifest-path "${MANIFEST_PATH}" --test pq3_compiler_executor_verifier_closed_loop_e2e
run_step "decision-trace-four-state" cargo test --manifest-path "${MANIFEST_PATH}" --lib requirement_swarm_emits_accept_repair_reject_escalate_decisions_in_same_session
run_step "e2e-replay-mismatch-explainer" cargo test --manifest-path "${MANIFEST_PATH}" --test p10_replay_mismatch_explainer_e2e
run_step "e2e-intent-query-tools-compact-verify-snapshot-resume-replay" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e
run_step "e2e-no-bypass-gate" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_no_bypass_gate_e2e
run_step "e2e-no-bypass-static-scan-business-layers" cargo test --manifest-path "${MANIFEST_PATH}" --test no_bypass_static_scan no_bypass_static_scan_business_layers
run_step "e2e-no-bypass-kernel" cargo test --manifest-path "${MANIFEST_PATH}" --test p5_runtime_escape_guard
run_step "e2e-no-bypass-mediator" cargo test --manifest-path "${MANIFEST_PATH}" --test mediator_no_bypass_e2e
run_step "artifact-gate-write-evidence-required" cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_requires_write_evidence_even_if_file_exists
run_step "artifact-gate-fake-success-rejected" cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_rejects_fake_success_when_proof_hash_mismatch
run_step "budget-preflight-and-ledger-hard-check" cargo test --manifest-path "${MANIFEST_PATH}" --test p8_budget_ledger_sovereignty
run_step "recovery-drill-chaos-recorded" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_chaos_case_records_failover
run_step "recovery-drill-mttr-recorded" cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_recover_marks_failover_with_mttr
run_step "d11-parallel-tool-call-events" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_query_loop_parallel_tool_events_contract
run_step "d11-two-stage-compact" cargo test --manifest-path "${MANIFEST_PATH}" --test pq2_compaction_resume_boundary
run_step "d11-named-snapshot-transcript" cargo test --manifest-path "${MANIFEST_PATH}" --test pq7_session_named_snapshot_transcript_e2e
run_step "d11-background-task-manager" cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_background_task_manager_e2e
run_step "d11-mcp-manager-service-spine" cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_service_mediation_spine
run_step "d11-aggregate-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_d11_compact_snapshot_task_mcp_parallel_e2e
run_step "d12-storage-postgres-wal-dualwrite-replay" cargo test --manifest-path "${MANIFEST_PATH}" --test d12_storage_postgres_wal_dualwrite_replay_e2e
run_step "rollout-gating-test" cargo test --manifest-path "${MANIFEST_PATH}" --test p6_rollout_gating
run_step "sandbox-acceptance" bash ./deploy/scripts/sandbox_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-sandbox"
run_step "pwiki-acceptance" bash ./deploy/scripts/pwiki_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-pwiki"
run_step "pwiki11-acceptance" bash ./deploy/scripts/pwiki11_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pwiki11"
run_step "pq11-skill-foundry-acceptance" bash ./deploy/scripts/pq11_skill_foundry_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pq11"
run_step "pevo-evolution-acceptance" bash ./deploy/scripts/pevo_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pevo"
run_step "d46-slo-acceptance" bash ./deploy/scripts/d46_slo_acceptance.sh "${MANIFEST_PATH}"
run_step "ops-acceptance" bash ./deploy/scripts/ops_acceptance.sh "${MANIFEST_PATH}" "./deploy/config/autoloop.dev.toml" "${SESSION_PREFIX}-ops"
run_step "signal-acceptance" bash ./deploy/scripts/signal_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-signal"
run_step "frontend-cli-acceptance" bash ./deploy/scripts/frontend_cli_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-frontend"
run_step "d14-storage-cutover-acceptance" bash ./deploy/scripts/d14_storage_cutover_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-d14"
run_step "rollback-daily-drill" bash ./deploy/scripts/rollback_daily_drill.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-rb"

set_local_state_store

for stage in shadow canary10 canary30 full; do
  case "${stage}" in
    shadow) mode="shadow"; ratio="0.2"; session="${SESSION_PREFIX}-shadow" ;;
    canary10) mode="canary"; ratio="0.1"; session="${SESSION_PREFIX}-canary10" ;;
    canary30) mode="canary"; ratio="0.3"; session="${SESSION_PREFIX}-canary30" ;;
    full) mode="full"; ratio="1.0"; session="${SESSION_PREFIX}-full" ;;
  esac

  set_gate_config "${mode}" "${ratio}"
  run_step "rollout-${stage}-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
  run_step "rollout-${stage}-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session}" system health
  if [[ "${stage}" == "shadow" ]]; then
    run_artifact_shadow "${SESSION_PREFIX}-artifact-shadow"
    run_benchmark_shadow "${SESSION_PREFIX}-benchmark-shadow"
  fi
done

run_step "replay-report-export" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-full" system replay-report --output "${REPLAY_OUT}"

if [[ ! -f "${REPLAY_OUT}" ]]; then
  echo "Replay report file was not generated: ${REPLAY_OUT}" >> "${LOG_PATH}"
  exit 1
fi

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
  "replay_report": "${REPLAY_OUT}",
  "benchmark_report": "${BENCHMARK_OUT}",
  "benchmark_compare_report": "${BENCHMARK_COMPARE_OUT}",
  "all_passed": true,
  "required_checks": [
    "intent-execute-verify-persist-replay",
    "decision-trace-four-state",
    "replay-mismatch-explainer",
    "no-bypass-kernel-mediator-static",
    "artifact-hard-gate-and-proof",
    "budget-preflight-ledger",
    "recovery-drill",
    "pwiki11-acceptance",
"pevo-evolution-acceptance",
"d46-slo-acceptance",
"ops-acceptance",
"signal-acceptance",
    "frontend-cli-acceptance",
    "d12-storage-postgres-wal-dualwrite-replay",
    "rollback-daily-drill",
    "d12-real-benchmark-run-and-compare"
  ],
  "rollout": ["shadow", "10%", "30%", "full", "rollback"],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "WEEK6_ACCEPTANCE_OK log=${LOG_PATH}"
echo "WEEK6_ACCEPTANCE_JSON=${JSON_PATH}"








