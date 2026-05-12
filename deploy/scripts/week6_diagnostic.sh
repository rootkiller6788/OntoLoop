#!/usr/bin/env bash
set -uo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-week6-diagnostic}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/week6-diagnostic-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/week6-diagnostic-report-${STAMP}.json"
CANONICAL_JSON_PATH="${RUNTIME_DIR}/week6_diagnostic_report.json"
mkdir -p "${RUNTIME_DIR}"

PREV_AUTOLOOP_PROFILE="${AUTOLOOP_PROFILE-}"
export AUTOLOOP_PROFILE="production-e2e"
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

restore_profile() {
  if [[ -n "${PREV_AUTOLOOP_PROFILE}" ]]; then
    export AUTOLOOP_PROFILE="${PREV_AUTOLOOP_PROFILE}"
  else
    unset AUTOLOOP_PROFILE || true
  fi
}
trap restore_profile EXIT

RESULTS_JSON=""

add_result() {
  local check_id="$1"
  local cmd="$2"
  local passed="$3"
  local exit_code="$4"
  local duration_ms="$5"
  local err_msg="$6"
  local escaped_cmd escaped_err
  escaped_cmd="${cmd//\\/\\\\}"
  escaped_cmd="${escaped_cmd//\"/\\\"}"
  escaped_cmd="${escaped_cmd//$'\n'/\\n}"
  escaped_cmd="${escaped_cmd//$'\r'/\\r}"
  escaped_err="${err_msg//\\/\\\\}"
  escaped_err="${escaped_err//\"/\\\"}"
  escaped_err="${escaped_err//$'\n'/\\n}"
  escaped_err="${escaped_err//$'\r'/\\r}"
  local stage severity
  case "${check_id}" in
    cargo-check|config-doctor*) stage="L0" ;;
    *no-bypass*|*artifact-gate*|*waltx*) stage="L1" ;;
    *sandbox*|*frontend*|*signal*|*pevo*|*d14*|*d12-storage*) stage="L2" ;;
    *) stage="L2" ;;
  esac
  if [[ "${passed}" == "true" ]]; then severity="info"; else severity="blocker"; fi
  RESULTS_JSON+="{\"stage\":\"${stage}\",\"check_id\":\"${check_id}\",\"passed\":${passed},\"severity\":\"${severity}\",\"deny_reason\":\"${escaped_err}\",\"evidence_ref\":null,\"replay_fp\":null,\"duration_ms\":${duration_ms}},"
}

run_step() {
  local name="$1"
  shift
  local cmd=("$@")
  local start end duration exit_code err
  start="$(date +%s%3N)"

  {
    echo ""
    echo "==== RUN: [${name}] ${cmd[*]} ===="
  } >> "${LOG_PATH}"

  (
    cd "${REPO_ROOT}"
    "${cmd[@]}"
  ) >> "${LOG_PATH}" 2>&1
  exit_code=$?

  end="$(date +%s%3N)"
  duration=$((end - start))
  err=""
  if [[ ${exit_code} -ne 0 ]]; then
    err="exit_code=${exit_code}"
    add_result "${name}" "${cmd[*]}" "false" "${exit_code}" "${duration}" "${err}"
  else
    add_result "${name}" "${cmd[*]}" "true" "0" "${duration}" ""
  fi
}

run_step "cargo-check" cargo check --workspace --manifest-path "${MANIFEST_PATH}"
run_step "p10-day10-acceptance" cargo test --manifest-path "${MANIFEST_PATH}" --test p10_day10_acceptance_e2e
run_step "pq3-closed-loop-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test pq3_compiler_executor_verifier_closed_loop_e2e
run_step "p10-replay-mismatch-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test p10_replay_mismatch_explainer_e2e
run_step "pq10-intent-query-chain-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e
run_step "pq10-no-bypass-gate-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_no_bypass_gate_e2e
run_step "pq3-permission-mode-tristate-matrix" cargo test --manifest-path "${MANIFEST_PATH}" --test pq3_permission_mode_matrix
run_step "no-bypass-static-scan" cargo test --manifest-path "${MANIFEST_PATH}" --test no_bypass_static_scan
run_step "d10-d11-security-governance" cargo test --manifest-path "${MANIFEST_PATH}" --test d10_d11_security_governance_gate_suite
run_step "p8-budget-ledger" cargo test --manifest-path "${MANIFEST_PATH}" --test p8_budget_ledger_sovereignty
run_step "d12-storage-postgres-wal" cargo test --manifest-path "${MANIFEST_PATH}" --test d12_storage_postgres_wal_dualwrite_replay_e2e
run_step "waltx-production-write-min" cargo test --manifest-path "${MANIFEST_PATH}" --test waltx_production_write_minimal_e2e
run_step "artifact-gate-write-evidence-required" cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_requires_write_evidence_even_if_file_exists
run_step "artifact-gate-fake-success-rejected" cargo test --manifest-path "${MANIFEST_PATH}" --lib artifact_gate_rejects_fake_success_when_proof_hash_mismatch
run_step "config-doctor-bad-config-blocked" cargo test --manifest-path "${MANIFEST_PATH}" --bins system_config_doctor_blocks_intentionally_bad_config
run_step "sandbox-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/sandbox_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}" -ProdConfigPath "${PROD_CONFIG_PATH}" -SessionPrefix "${SESSION_PREFIX}-sandbox"
run_step "signal-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/signal_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}" -SessionPrefix "${SESSION_PREFIX}-signal"
run_step "frontend-cli-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/frontend_cli_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}" -SessionPrefix "${SESSION_PREFIX}-frontend"
run_step "pevo-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/pevo_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}" -ProdConfigPath "${PROD_CONFIG_PATH}" -SessionPrefix "${SESSION_PREFIX}-pevo"
run_step "version-a-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/version_a_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}"
run_step "d14-rollout-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/d14_rollout.ps1 -ManifestPath "${MANIFEST_PATH}" -ProdConfigPath "${PROD_CONFIG_PATH}" -SessionPrefix "${SESSION_PREFIX}-d14"
run_step "d46-slo-acceptance" powershell -ExecutionPolicy Bypass -File ./deploy/scripts/d46_slo_acceptance.ps1 -ManifestPath "${MANIFEST_PATH}"

RESULTS_JSON="[${RESULTS_JSON%,}]"
RESULTS_TMP_PATH="${RUNTIME_DIR}/week6-diagnostic-results-${STAMP}.json"
printf '%s\n' "${RESULTS_JSON}" > "${RESULTS_TMP_PATH}"
RESULTS_TMP_PATH_WIN="$(cygpath -w "${RESULTS_TMP_PATH}")"
JSON_PATH_WIN="$(cygpath -w "${JSON_PATH}")"
CANONICAL_JSON_PATH_WIN="$(cygpath -w "${CANONICAL_JSON_PATH}")"
LOG_PATH_WIN="$(cygpath -w "${LOG_PATH}")"
REPO_ROOT_WIN="$(cygpath -w "${REPO_ROOT}")"

powershell -NoProfile -Command "\
\$results = Get-Content -Raw -Path '${RESULTS_TMP_PATH_WIN}' | ConvertFrom-Json; \
if (\$null -eq \$results) { \$results = @() }; \
\$arr = @(\$results); \
\$failed = @(\$arr | Where-Object { -not \$_.passed } | ForEach-Object { [string]\$_.check_id }); \
\$payload = [ordered]@{ \
  generated_at = [DateTime]::UtcNow.ToString('s') + 'Z'; \
  profile = 'production-e2e'; \
  repo_root = '${REPO_ROOT_WIN}'; \
  manifest = '${MANIFEST_PATH}'; \
  prod_config = '${PROD_CONFIG_PATH}'; \
  all_passed = (\$failed.Count -eq 0); \
  total = \$arr.Count; \
  passed = (@(\$arr | Where-Object { \$_.passed }).Count); \
  failed = \$failed.Count; \
  blockers = \$failed; \
  checks = \$arr; \
  log_path = '${LOG_PATH_WIN}'; \
}; \
\$json = \$payload | ConvertTo-Json -Depth 8; \
Set-Content -Path '${JSON_PATH_WIN}' -Value \$json -Encoding utf8; \
Set-Content -Path '${CANONICAL_JSON_PATH_WIN}' -Value \$json -Encoding utf8 \
"

echo "WEEK6_DIAGNOSTIC_JSON=${JSON_PATH}"
echo "WEEK6_DIAGNOSTIC_CANONICAL_JSON=${CANONICAL_JSON_PATH}"
echo "WEEK6_DIAGNOSTIC_LOG=${LOG_PATH}"
