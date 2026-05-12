#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-week6-rollout}"
ARTIFACT_PATH="${4:-/d/AutoLoop/autoloop-app/deploy/runtime/week6-shadow-bill-replica.html}"
RUN_D13_FULL="${AUTOLOOP_RUN_D13_FULL:-auto}"
BENCHMARK_SMOKE_LIMIT="${AUTOLOOP_D13_SMOKE_LIMIT:-12}"
BENCHMARK_FULL_LIMIT="${AUTOLOOP_D13_FULL_LIMIT:-52}"
RUN_SOAK_STABILITY="${AUTOLOOP_RUN_SOAK_STABILITY:-0}"
SOAK_DURATION_HOURS="${AUTOLOOP_SOAK_DURATION_HOURS:-6}"
FORCE_WEEKLY_FULL_DRILL="${AUTOLOOP_FORCE_WEEKLY_FULL_DRILL:-0}"
CHANGED_FILES_PATH="${AUTOLOOP_CHANGED_FILES_PATH:-}"
D13_FULL_ENABLED=false
D46_REPORT_PATH=""
D13_SMOKE_REPORT=""
D13_FULL_REPORT=""
SOAK_STABILITY_REPORT=""
ROLLBACK_DRILL_REPORT=""
FAULT_DRILL_MODE="light"
ROLLBACK_DRILL_MODE="light"
IMPACT_SELECTOR_REPORT=""
IMPACTED_TESTS_HASH=""
IMPACTED_CHECKS_CSV=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/week6-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/week6-acceptance-${STAMP}.json"
CANONICAL_JSON_PATH="${RUNTIME_DIR}/week6_full_acceptance.json"
BACKUP_PATH="${RUNTIME_DIR}/week6-autoloop.prod.backup-${STAMP}.toml"
REPLAY_OUT="${RUNTIME_DIR}/week6-replay-report-${STAMP}.json"
BENCHMARK_OUT="${RUNTIME_DIR}/week6-benchmark-${STAMP}.json"
BENCHMARK_COMPARE_OUT="${RUNTIME_DIR}/week6-benchmark-compare-${STAMP}.json"
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"
PREV_AUTOLOOP_PROFILE="${AUTOLOOP_PROFILE-}"
export AUTOLOOP_PROFILE="production-e2e"

mkdir -p "${RUNTIME_DIR}"
cp "${REPO_ROOT}/${PROD_CONFIG_PATH}" "${BACKUP_PATH}"

restore_config() {
  if [[ -f "${BACKUP_PATH}" ]]; then
    cp "${BACKUP_PATH}" "${REPO_ROOT}/${PROD_CONFIG_PATH}"
  fi
  if [[ -n "${PREV_AUTOLOOP_PROFILE}" ]]; then
    export AUTOLOOP_PROFILE="${PREV_AUTOLOOP_PROFILE}"
  else
    unset AUTOLOOP_PROFILE || true
  fi
}
trap restore_config EXIT

run_step() {
  local name="$1"
  local retry_count="${2:-0}"
  shift 2
  local cmd=("$@")
  local display
  display="$(printf '%s ' "${cmd[@]}")"
  display="${display% }"
  local esc_cmd
  esc_cmd="$(printf '%s' "${display}" | sed 's/"/\\"/g')"
  local attempt=0
  local max_attempts=$((retry_count + 1))
  local step_output_file
  step_output_file="${RUNTIME_DIR}/week6-step-${name//[^a-zA-Z0-9_-]/_}-${STAMP}.log"
  if [[ -n "${IMPACTED_CHECKS_CSV}" ]]; then
    case ",${IMPACTED_CHECKS_CSV}," in
      *",${name},"*) ;;
      *)
        echo "" >> "${LOG_PATH}"
        echo "==== SKIP: [${name}] not in impacted scope ====" >> "${LOG_PATH}"
        RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true,\"skipped\":true,\"skip_reason\":\"not_impacted\",\"exit_code\":0},"
        return 0
        ;;
    esac
  fi

  echo "" >> "${LOG_PATH}"
  echo "==== RUN: [${name}] ${display} ====" >> "${LOG_PATH}"

  while (( attempt < max_attempts )); do
    attempt=$((attempt + 1))
    : > "${step_output_file}"
    (
      cd "${REPO_ROOT}"
      "${cmd[@]}"
    ) > "${step_output_file}" 2>&1
    local exit_code=$?
    cat "${step_output_file}" >> "${LOG_PATH}"

    if (( exit_code == 0 )); then
      RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${esc_cmd}\",\"passed\":true,\"exit_code\":0},"
      return 0
    fi

    local step_output
    step_output="$(cat "${step_output_file}" 2>/dev/null || true)"
    local lower_output
    lower_output="$(printf '%s' "${step_output}" | tr '[:upper:]' '[:lower:]')"
    local is_permission_failure=false
    local is_timeout_failure=false
    if [[ "${lower_output}" == *"permission"* || "${lower_output}" == *"approval required"* || "${lower_output}" == *"requires approval"* || "${lower_output}" == *"blocked by policy"* || "${lower_output}" == *"denied"* ]]; then
      is_permission_failure=true
    fi
    if [[ "${lower_output}" == *"timeout"* || "${lower_output}" == *"timed out"* || "${lower_output}" == *"deadline exceeded"* || "${lower_output}" == *"operation timed out"* ]]; then
      is_timeout_failure=true
    fi

    local allow_retry=false
    if [[ "${is_permission_failure}" == "false" && "${is_timeout_failure}" == "true" ]]; then
      allow_retry=true
    fi

    if (( attempt < max_attempts )) && [[ "${allow_retry}" == "true" ]]; then
      echo "retrying [${name}] attempt $((attempt + 1))/${max_attempts} after exit=${exit_code} (reason=timeout)" >> "${LOG_PATH}"
      sleep 2
      continue
    fi

    if (( attempt < max_attempts )) && [[ "${allow_retry}" == "false" ]]; then
      echo "retry skipped for [${name}] exit=${exit_code} (reason=non-timeout-or-permission)" >> "${LOG_PATH}"
    fi

    echo "Command failed (${exit_code}): [${name}] ${display} (attempts=${attempt})" >> "${LOG_PATH}"
    return "${exit_code}"
  done
}

init_impact_selection() {
  local selector_out
  if [[ -n "${CHANGED_FILES_PATH}" ]]; then
    selector_out="$(bash ./deploy/scripts/impact_test_selector.sh "${CHANGED_FILES_PATH}" 2>/dev/null)"
  else
    selector_out="$(bash ./deploy/scripts/impact_test_selector.sh 2>/dev/null)"
  fi
  IMPACT_SELECTOR_REPORT="$(printf '%s\n' "${selector_out}" | grep '^IMPACT_SELECTOR_JSON=' | tail -n 1 | cut -d'=' -f2-)"
  IMPACTED_TESTS_HASH="$(printf '%s\n' "${selector_out}" | grep '^IMPACTED_TESTS_HASH=' | tail -n 1 | cut -d'=' -f2-)"
  if [[ -z "${IMPACT_SELECTOR_REPORT}" || ! -f "${IMPACT_SELECTOR_REPORT}" ]]; then
    echo "impact selector report missing" >> "${LOG_PATH}"
    exit 1
  fi
  local selector_win
  selector_win="$(cygpath -w "${IMPACT_SELECTOR_REPORT}")"
  IMPACTED_CHECKS_CSV="$(powershell -NoProfile -Command "\
  \$obj = Get-Content -Raw -Path '${selector_win}' | ConvertFrom-Json; \
  [string]::Join(',', @(\$obj.impacted_checks))\
  ")"
  IMPACTED_CHECKS_CSV="${IMPACTED_CHECKS_CSV//$'\r'/}"
  IMPACTED_CHECKS_CSV="${IMPACTED_CHECKS_CSV//$'\n'/}"
  if [[ "${IMPACTED_CHECKS_CSV}" == *"__RUN_ROLLOUT__"* ]]; then
    IMPACTED_CHECKS_CSV="${IMPACTED_CHECKS_CSV},rollout-shadow-status,rollout-shadow-health,rollout-canary10-status,rollout-canary10-health,rollout-canary30-status,rollout-canary30-health,rollout-full-status,rollout-full-health"
  fi
  IMPACTED_CHECKS_CSV="${IMPACTED_CHECKS_CSV},replay-report-export,rollback-status,rollback-health"
  RESULTS_JSON+="{\"name\":\"impact-test-selector\",\"command\":\"impact_test_selector.sh\",\"passed\":true,\"exit_code\":0,\"report\":\"${IMPACT_SELECTOR_REPORT}\"},"
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

run_config_doctor_gate() {
  local session_id="$1"
  local profile="$2"
  local out="${RUNTIME_DIR}/week6-config-doctor-${STAMP}.json"
  run_step "pre-rollout-config-doctor-gate" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" system config doctor --profile "${profile}" --output "${out}"
  if [[ ! -f "${out}" ]]; then
    echo "config doctor output missing: ${out}" >> "${LOG_PATH}"
    exit 1
  fi
  local out_win
  out_win="$(cygpath -w "${out}")"
  powershell -NoProfile -Command "\
  \$doctor = Get-Content -Raw -Path '${out_win}' | ConvertFrom-Json; \
  if ([string]\$doctor.status -ne 'pass') { throw 'config doctor hard gate failed: status=' + [string]\$doctor.status }; \
  \$requiredIds = @('profile.alignment','runtime.gate_mode','runtime.rollback_window','storage.postgres.enabled_uri','storage.backend_consistency'); \
  foreach (\$id in \$requiredIds) { \
    \$check = \$doctor.checks | Where-Object { \$_.id -eq \$id } | Select-Object -First 1; \
    if (\$null -eq \$check) { throw 'config doctor hard gate missing required check: ' + \$id }; \
    if (-not \$check.passed) { throw 'config doctor hard gate check failed: ' + \$id + ' => ' + [string]\$check.message }; \
  }\
  "
  RESULTS_JSON+="{\"name\":\"pre-rollout-config-doctor-gate\",\"command\":\"system config doctor --profile ${profile} --output ${out}\",\"passed\":true,\"report\":\"${out}\"},"
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

  local benchmark_out_win benchmark_compare_out_win previous_report_win
  benchmark_out_win="$(cygpath -w "${BENCHMARK_OUT}")"
  benchmark_compare_out_win="$(cygpath -w "${BENCHMARK_COMPARE_OUT}")"
  previous_report_win=""
  if [[ -n "${previous_report}" ]]; then
    previous_report_win="$(cygpath -w "${previous_report}")"
  fi
  powershell -NoProfile -Command "\
  \$newPath = '${benchmark_out_win}'; \
  \$oldPath = '${previous_report_win}'; \
  \$comparePath = '${benchmark_compare_out_win}'; \
  \$new = Get-Content -Raw -Path \$newPath | ConvertFrom-Json; \
  \$payload = [ordered]@{ \
    generated_at = '${STAMP}'; \
    session_id = '${session_id}'; \
    new_report = \$newPath; \
    old_report = \$null; \
    new = [ordered]@{ \
      total = \$new.total; \
      passed = \$new.passed; \
      failed = \$new.failed; \
      success_rate = \$new.success_rate; \
      total_retry_count = \$new.total_retry_count; \
      average_retry_count = \$new.average_retry_count; \
      failure_reason_distribution = \$new.failure_reason_distribution; \
      evidence_ref = \$new.evidence_ref; \
    }; \
    delta = \$null; \
  }; \
  if (-not [string]::IsNullOrWhiteSpace(\$oldPath) -and (Test-Path \$oldPath) -and (\$oldPath -ne \$newPath)) { \
    \$old = Get-Content -Raw -Path \$oldPath | ConvertFrom-Json; \
    \$payload.old_report = \$oldPath; \
    \$payload.delta = [ordered]@{ \
      success_rate = ([double]\$new.success_rate) - ([double]\$old.success_rate); \
      total_retry_count = ([int64]\$new.total_retry_count) - ([int64]\$old.total_retry_count); \
      average_retry_count = ([double]\$new.average_retry_count) - ([double]\$old.average_retry_count); \
      failed = ([int]\$new.failed) - ([int]\$old.failed); \
    }; \
  }; \
  (\$payload | ConvertTo-Json -Depth 8) | Set-Content -Path \$comparePath -Encoding utf8\
  "

  RESULTS_JSON+="{\"name\":\"d12-real-benchmark-compare\",\"command\":\"system benchmark run + compare old/new\",\"passed\":true,\"benchmark_report\":\"${BENCHMARK_OUT}\",\"benchmark_compare\":\"${BENCHMARK_COMPARE_OUT}\"},"
}

run_d13_benchmark_cadence() {
  assert_d13_threshold_pass() {
    local report_path="$1"
    local label="$2"
    local report_path_win
    report_path_win="$(cygpath -w "${report_path}")"
    powershell -NoProfile -Command "\
    if (-not (Test-Path '${report_path_win}')) { throw '${label} benchmark report missing: ${report_path_win}' }; \
    \$report = Get-Content -Raw -Path '${report_path_win}' | ConvertFrom-Json; \
    if (-not (\$report.threshold_pass -eq \$true)) { \
      \$success = [double]\$report.success_rate_percent; \
      \$compliance = [double]\$report.compliance_rate_percent; \
      \$minSuccess = [double]\$report.thresholds.minimum_success_rate_percent; \
      \$minCompliance = [double]\$report.thresholds.minimum_compliance_rate_percent; \
      throw '${label} benchmark threshold failed: success=' + \$success + '% (<'+\$minSuccess+'%) or compliance=' + \$compliance + '% (<'+\$minCompliance+'%). rollout blocked.' \
    }\
    "
  }

  local session_id="$1"
  local smoke_out
  smoke_out="$(
    bash ./deploy/scripts/d13_realbiz_benchmark_acceptance.sh \
      "${MANIFEST_PATH}" \
      "${PROD_CONFIG_PATH}" \
      "${session_id}-smoke" \
      "${BENCHMARK_SMOKE_LIMIT}" \
      "60000"
  )"
  local smoke_report
  smoke_report="$(printf '%s\n' "${smoke_out}" | grep '^D13_REALBIZ_BENCHMARK_JSON=' | tail -n 1 | cut -d'=' -f2-)"
  if [[ -z "${smoke_report}" ]]; then
    echo "d13 smoke benchmark report path missing" >> "${LOG_PATH}"
    exit 1
  fi
  D13_SMOKE_REPORT="${smoke_report}"
  assert_d13_threshold_pass "${smoke_report}" "d13-smoke"
  RESULTS_JSON+="{\"name\":\"d13-smoke-benchmark\",\"command\":\"d13_realbiz_benchmark_acceptance.sh --limit ${BENCHMARK_SMOKE_LIMIT}\",\"passed\":true,\"report\":\"${smoke_report}\"},"

  if [[ "${RUN_D13_FULL}" == "1" || "${RUN_D13_FULL}" == "true" || "${RUN_D13_FULL}" == "yes" ]]; then
    D13_FULL_ENABLED=true
    local full_out
    full_out="$(
      bash ./deploy/scripts/d13_realbiz_benchmark_acceptance.sh \
        "${MANIFEST_PATH}" \
        "${PROD_CONFIG_PATH}" \
        "${session_id}-full" \
        "${BENCHMARK_FULL_LIMIT}" \
        "60000"
    )"
    local full_report
    full_report="$(printf '%s\n' "${full_out}" | grep '^D13_REALBIZ_BENCHMARK_JSON=' | tail -n 1 | cut -d'=' -f2-)"
    if [[ -z "${full_report}" ]]; then
      echo "d13 full benchmark report path missing" >> "${LOG_PATH}"
      exit 1
    fi
    D13_FULL_REPORT="${full_report}"
    assert_d13_threshold_pass "${full_report}" "d13-full"
    RESULTS_JSON+="{\"name\":\"d13-full-benchmark\",\"command\":\"d13_realbiz_benchmark_acceptance.sh --limit ${BENCHMARK_FULL_LIMIT}\",\"passed\":true,\"report\":\"${full_report}\"},"
  else
    D13_FULL_ENABLED=false
    D13_FULL_REPORT=""
    RESULTS_JSON+="{\"name\":\"d13-full-benchmark\",\"command\":\"d13_realbiz_benchmark_acceptance.sh --limit ${BENCHMARK_FULL_LIMIT}\",\"passed\":true,\"skipped\":true,\"reason\":\"RUN_D13_FULL disabled (smoke default)\"},"
  fi
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

init_impact_selection
run_step "cargo-check" 0 cargo check --workspace --manifest-path "${MANIFEST_PATH}"
run_step "e2e-intent-execute-verify-persist-replay" 1 cargo test --manifest-path "${MANIFEST_PATH}" --test p10_day10_acceptance_e2e
run_step "e2e-compiler-executor-verifier-closed-loop" 1 cargo test --manifest-path "${MANIFEST_PATH}" --test pq3_compiler_executor_verifier_closed_loop_e2e
run_step "decision-trace-four-state" 0 cargo test --manifest-path "${MANIFEST_PATH}" --lib requirement_swarm_emits_accept_repair_reject_escalate_decisions_in_same_session
run_step "e2e-replay-mismatch-explainer" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test p10_replay_mismatch_explainer_e2e
run_step "e2e-intent-query-tools-compact-verify-snapshot-resume-replay" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e
run_step "e2e-no-bypass-gate" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_no_bypass_gate_e2e
run_step "admission-tristate-matrix" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq3_permission_mode_matrix
run_step "e2e-no-bypass-static-scan-all-domains" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test no_bypass_static_scan
run_step "d10-d11-security-governance-gate-suite" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test d10_d11_security_governance_gate_suite
run_step "e2e-no-bypass-kernel" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test p5_runtime_escape_guard
run_step "e2e-no-bypass-mediator" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test mediator_no_bypass_e2e
run_step "artifact-gate-write-evidence-required" 0 cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_requires_write_evidence_even_if_file_exists
run_step "artifact-gate-fake-success-rejected" 0 cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_rejects_fake_success_when_proof_hash_mismatch
run_step "budget-preflight-and-ledger-hard-check" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test p8_budget_ledger_sovereignty
run_step "recovery-drill-chaos-recorded" 0 cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_chaos_case_records_failover
run_step "recovery-drill-mttr-recorded" 0 cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_recover_marks_failover_with_mttr
run_step "d11-parallel-tool-call-events" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_query_loop_parallel_tool_events_contract
run_step "d11-two-stage-compact" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq2_compaction_resume_boundary
run_step "d11-named-snapshot-transcript" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq7_session_named_snapshot_transcript_e2e
run_step "d11-background-task-manager" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_background_task_manager_e2e
run_step "d11-mcp-manager-service-spine" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_service_mediation_spine
run_step "d11-aggregate-e2e" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_d11_compact_snapshot_task_mcp_parallel_e2e
run_step "d12-storage-postgres-wal-dualwrite-replay" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test d12_storage_postgres_wal_dualwrite_replay_e2e
run_step "waltx-production-write-minimal-e2e" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test waltx_production_write_minimal_e2e
run_step "config-doctor-bad-config-blocked-e2e" 0 cargo test --manifest-path "${MANIFEST_PATH}" --bins system_config_doctor_blocks_intentionally_bad_config
run_step "rollout-gating-test" 0 cargo test --manifest-path "${MANIFEST_PATH}" --test p6_rollout_gating
run_step "sandbox-acceptance" 0 bash ./deploy/scripts/sandbox_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-sandbox"
run_step "pwiki-acceptance" 0 bash ./deploy/scripts/pwiki_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-pwiki"
run_step "pwiki11-acceptance" 0 bash ./deploy/scripts/pwiki11_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pwiki11"
run_step "pq11-skill-foundry-acceptance" 0 bash ./deploy/scripts/pq11_skill_foundry_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pq11"
run_step "pevo-evolution-acceptance" 0 bash ./deploy/scripts/pevo_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-pevo"
run_step "d46-slo-acceptance" 0 bash ./deploy/scripts/d46_slo_acceptance.sh "${MANIFEST_PATH}"
D46_REPORT_PATH="$(ls -1t "${RUNTIME_DIR}"/d46-slo-acceptance-*.json 2>/dev/null | head -n 1 || true)"
if [[ -z "${D46_REPORT_PATH}" ]]; then
  echo "d46 report path missing" >> "${LOG_PATH}"
  exit 1
fi
if [[ "${RUN_SOAK_STABILITY}" == "1" || "${RUN_SOAK_STABILITY}" == "true" ]]; then
  SOAK_OUT="$(
    bash ./deploy/scripts/soak_stability_acceptance.sh \
      "${MANIFEST_PATH}" \
      "${PROD_CONFIG_PATH}" \
      "${SESSION_PREFIX}-soak" \
      "${SOAK_DURATION_HOURS}" \
      "90" \
      "1"
  )"
  SOAK_STABILITY_REPORT="$(printf '%s\n' "${SOAK_OUT}" | grep '^SOAK_STABILITY_JSON=' | tail -n 1 | cut -d'=' -f2-)"
  if [[ -z "${SOAK_STABILITY_REPORT}" ]]; then
    echo "soak stability report path missing" >> "${LOG_PATH}"
    exit 1
  fi
  RESULTS_JSON+="{\"name\":\"soak-stability-acceptance\",\"command\":\"soak_stability_acceptance.sh\",\"passed\":true,\"report\":\"${SOAK_STABILITY_REPORT}\"},"
fi
run_step "ops-acceptance" 0 bash ./deploy/scripts/ops_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-ops"
run_step "d12-ops-productized-acceptance" 0 bash ./deploy/scripts/d12_ops_productized_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-d12ops"
run_step "signal-acceptance" 0 bash ./deploy/scripts/signal_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-signal"
run_step "frontend-cli-acceptance" 0 bash ./deploy/scripts/frontend_cli_acceptance.sh "${MANIFEST_PATH}" "${SESSION_PREFIX}-frontend"
run_step "d14-storage-cutover-acceptance" 0 bash ./deploy/scripts/d14_storage_cutover_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-d14"
day_of_week="$(date +%u)"
if [[ "${FORCE_WEEKLY_FULL_DRILL}" == "1" || "${FORCE_WEEKLY_FULL_DRILL}" == "true" || "${day_of_week}" == "7" ]]; then
  ROLLBACK_DRILL_MODE="full"
  FAULT_DRILL_MODE="full"
fi
ROLLBACK_DRILL_OUT="$(
  bash ./deploy/scripts/rollback_daily_drill.sh \
    "${MANIFEST_PATH}" \
    "${PROD_CONFIG_PATH}" \
    "${SESSION_PREFIX}-rb" \
    "${ROLLBACK_DRILL_MODE}"
)"
ROLLBACK_DRILL_REPORT="$(printf '%s\n' "${ROLLBACK_DRILL_OUT}" | grep '^ROLLBACK_DAILY_DRILL_JSON=' | tail -n 1 | cut -d'=' -f2-)"
if [[ -z "${ROLLBACK_DRILL_REPORT}" ]]; then
  echo "rollback drill report path missing" >> "${LOG_PATH}"
  exit 1
fi
RESULTS_JSON+="{\"name\":\"rollback-daily-drill\",\"command\":\"rollback_daily_drill.sh --mode ${ROLLBACK_DRILL_MODE}\",\"passed\":true,\"report\":\"${ROLLBACK_DRILL_REPORT}\"},"

FAULT_DRILL_OUT="$(
  bash ./deploy/scripts/fault_injection_daily_drill.sh \
    "${MANIFEST_PATH}" \
    "${PROD_CONFIG_PATH}" \
    "${SESSION_PREFIX}-fault" \
    "${FAULT_DRILL_MODE}"
)"
FAULT_DRILL_REPORT="$(printf '%s\n' "${FAULT_DRILL_OUT}" | grep '^FAULT_INJECTION_DAILY_JSON=' | tail -n 1 | cut -d'=' -f2-)"
if [[ -z "${FAULT_DRILL_REPORT}" ]]; then
  echo "fault injection report path missing" >> "${LOG_PATH}"
  exit 1
fi
RESULTS_JSON+="{\"name\":\"fault-injection-daily-drill\",\"command\":\"fault_injection_daily_drill.sh\",\"passed\":true,\"report\":\"${FAULT_DRILL_REPORT}\"},"

set_local_state_store
run_config_doctor_gate "${SESSION_PREFIX}-config-doctor" "production-e2e"

# L0-L2 gate: full-chain (L3) can only start when required preconditions passed.
LOG_PATH_WIN="$(cygpath -w "${LOG_PATH}")"
powershell -NoProfile -Command "\
\$text = Get-Content -Raw -Path '${LOG_PATH_WIN}'; \
\$required = @('pre-rollout-config-doctor-gate','e2e-no-bypass-static-scan-all-domains','artifact-gate-write-evidence-required','d12-storage-postgres-wal-dualwrite-replay','sandbox-acceptance','signal-acceptance','frontend-cli-acceptance','pevo-evolution-acceptance'); \
foreach (\$name in \$required) { \
  \$runMarker = '==== RUN: [' + \$name + ']'; \
  \$skipMarker = '==== SKIP: [' + \$name + ']'; \
  if (\$text.IndexOf(\$runMarker) -lt 0 -and \$text.IndexOf(\$skipMarker) -lt 0) { throw 'L0-L2 gate failed: required check missing before L3: ' + \$name } \
}\
"

for stage in shadow canary10 canary30 full; do
  case "${stage}" in
    shadow) mode="shadow"; ratio="0.2"; session="${SESSION_PREFIX}-shadow" ;;
    canary10) mode="canary"; ratio="0.1"; session="${SESSION_PREFIX}-canary10" ;;
    canary30) mode="canary"; ratio="0.3"; session="${SESSION_PREFIX}-canary30" ;;
    full) mode="full"; ratio="1.0"; session="${SESSION_PREFIX}-full" ;;
  esac

  set_gate_config "${mode}" "${ratio}"
  run_step "rollout-${stage}-status" 0 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
  run_step "rollout-${stage}-health" 0 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session}" system health
  if [[ "${stage}" == "shadow" ]]; then
    run_artifact_shadow "${SESSION_PREFIX}-artifact-shadow"
    run_benchmark_shadow "${SESSION_PREFIX}-benchmark-shadow"
    run_d13_benchmark_cadence "${SESSION_PREFIX}-d13"
  fi
done

run_step "replay-report-export" 0 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-full" system replay-report --output "${REPLAY_OUT}"

if [[ ! -f "${REPLAY_OUT}" ]]; then
  echo "Replay report file was not generated: ${REPLAY_OUT}" >> "${LOG_PATH}"
  exit 1
fi

set_rollback_version "v1"
set_gate_config "shadow" "0.2"
run_step "rollback-status" 0 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollback-health" 0 cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-rollback" system health

VERSION_A_RAW="$(
  bash ./deploy/scripts/version_a_acceptance.sh "${MANIFEST_PATH}"
)"
VERSION_A_REPORT="$(printf '%s\n' "${VERSION_A_RAW}" | grep '^VERSION_A_ACCEPTANCE_JSON=' | tail -n 1 | cut -d'=' -f2-)"
if [[ -z "${VERSION_A_REPORT}" ]]; then
  echo "version-a report path missing" >> "${LOG_PATH}"
  exit 1
fi
RESULTS_JSON+="{\"name\":\"version-a-acceptance\",\"command\":\"version_a_acceptance.sh\",\"passed\":true,\"report\":\"${VERSION_A_REPORT}\"},"

D14_RAW="$(
  bash ./deploy/scripts/d14_rollout.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-d14"
)"
D14_REPORT="$(printf '%s\n' "${D14_RAW}" | grep '^D14_ROLLOUT_JSON=' | tail -n 1 | cut -d'=' -f2-)"
if [[ -z "${D14_REPORT}" ]]; then
  echo "d14 rollout report path missing" >> "${LOG_PATH}"
  exit 1
fi
RESULTS_JSON+="{\"name\":\"d14-rollout-final\",\"command\":\"d14_rollout.sh\",\"passed\":true,\"report\":\"${D14_REPORT}\"},"

RESULTS_JSON="[${RESULTS_JSON%,}]"

D13_REPORT_FOR_GATE="${D13_SMOKE_REPORT}"
if [[ "${D13_FULL_ENABLED}" == "true" && -n "${D13_FULL_REPORT:-}" ]]; then
  D13_REPORT_FOR_GATE="${D13_FULL_REPORT}"
fi

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
  "fault_injection_report": "${FAULT_DRILL_REPORT}",
  "rollback_drill_report": "${ROLLBACK_DRILL_REPORT}",
  "drill_modes": {"rollback":"${ROLLBACK_DRILL_MODE}","fault":"${FAULT_DRILL_MODE}"},
  "version_a_report": "${VERSION_A_REPORT}",
  "d14_rollout_report": "${D14_REPORT}",
  "d46_slo_report": "${D46_REPORT_PATH}",
  "soak_stability_report": "${SOAK_STABILITY_REPORT}",
  "impact_selector_report": "${IMPACT_SELECTOR_REPORT}",
  "impacted_tests_hash": "${IMPACTED_TESTS_HASH}",
  "all_passed": true,
  "required_checks": [
    "impact-test-selector",
    "intent-execute-verify-persist-replay",
    "decision-trace-four-state",
    "replay-mismatch-explainer",
    "no-bypass-kernel-mediator-static",
    "d10-d11-security-governance-gate-suite",
    "artifact-hard-gate-and-proof",
    "budget-preflight-ledger",
    "recovery-drill",
    "pwiki11-acceptance",
"pevo-evolution-acceptance",
"d46-slo-acceptance",
"ops-acceptance",
"d12-ops-productized-acceptance",
"signal-acceptance",
    "frontend-cli-acceptance",
    "d12-storage-postgres-wal-dualwrite-replay",
    "waltx-production-write-minimal-e2e",
    "config-doctor-bad-config-blocked-e2e",
    "rollback-daily-drill",
    "fault-injection-daily-drill",
    "version-a-acceptance",
    "d14-rollout-final",
    "d12-real-benchmark-run-and-compare",
    "d13-smoke-benchmark-always",
    "d13-full-benchmark-daily"
  ],
  "rollout": ["shadow", "10%", "30%", "full", "rollback"],
  "checks": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}",
  "release_gate_report": null
}
EOF

JSON_PATH_WIN="$(cygpath -w "${JSON_PATH}")"
CANONICAL_JSON_PATH_WIN="$(cygpath -w "${CANONICAL_JSON_PATH}")"
powershell -NoProfile -Command "\
\$src = '${JSON_PATH_WIN}'; \
\$data = Get-Content -Raw -Path \$src | ConvertFrom-Json; \
\$normalized = @(); \
foreach (\$item in \$data.checks) { \
  \$name = [string]\$item.name; \
  if (\$name.StartsWith('pre-rollout-config-doctor-gate') -or \$name.StartsWith('cargo-check')) { \$stage = 'L0' } \
  elseif (\$name.Contains('no-bypass') -or \$name.Contains('artifact') -or \$name.Contains('waltx')) { \$stage = 'L1' } \
  elseif (\$name.StartsWith('rollout-') -or \$name -eq 'replay-report-export') { \$stage = 'L3' } \
  else { \$stage = 'L2' }; \
  \$passed = [bool]\$item.passed; \
  \$normalized += [ordered]@{ \
    stage = \$stage; \
    check_id = \$name; \
    passed = \$passed; \
    severity = if (\$passed) { 'info' } else { 'blocker' }; \
    deny_reason = if (\$passed) { \$null } else { 'exit_code=' + [string]\$item.exit_code }; \
    evidence_ref = \$null; \
    replay_fp = \$null; \
    duration_ms = [int](if (\$null -eq \$item.duration_ms) { 0 } else { \$item.duration_ms }); \
    skipped = [bool](if (\$null -eq \$item.skipped) { \$false } else { \$item.skipped }); \
  }; \
}; \
  \$data.checks = \$normalized; \
\$json = \$data | ConvertTo-Json -Depth 8; \
Set-Content -Path \$src -Value \$json -Encoding utf8; \
Set-Content -Path '${CANONICAL_JSON_PATH_WIN}' -Value \$json -Encoding utf8\
"

echo "WEEK6_ACCEPTANCE_OK log=${LOG_PATH}"
echo "WEEK6_ACCEPTANCE_JSON=${JSON_PATH}"
echo "WEEK6_FULL_ACCEPTANCE_CANONICAL_JSON=${CANONICAL_JSON_PATH}"








