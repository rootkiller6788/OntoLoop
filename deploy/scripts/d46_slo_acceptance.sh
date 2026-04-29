#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
P95_THRESHOLD_MS="${2:-120000}"
ERROR_RATE_THRESHOLD="${3:-0.05}"
MTTR_THRESHOLD_MS="${4:-60000}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/d46-slo-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/d46-slo-acceptance-${STAMP}.json"
export RUST_MIN_STACK="${RUST_MIN_STACK:-33554432}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"

mkdir -p "${RUNTIME_DIR}"

STEPS_JSON=""
DURATIONS=()
TOTAL_STEPS=0
FAILED_STEPS=0
MTTR_MS=0

run_timed_step() {
  local name="$1"
  local category="$2"
  shift 2
  local cmd=("$@")

  TOTAL_STEPS=$((TOTAL_STEPS + 1))
  echo "" >> "${LOG_PATH}"
  echo "==== RUN: [${category}] [${name}] ${cmd[*]} ====" >> "${LOG_PATH}"

  local start_ms end_ms duration_ms exit_code
  start_ms="$(date +%s%3N)"
  set +e
  (
    cd "${REPO_ROOT}"
    "${cmd[@]}"
  ) >> "${LOG_PATH}" 2>&1
  exit_code=$?
  set -e
  end_ms="$(date +%s%3N)"
  duration_ms=$((end_ms - start_ms))
  DURATIONS+=("${duration_ms}")

  local passed="true"
  if [[ ${exit_code} -ne 0 ]]; then
    passed="false"
    FAILED_STEPS=$((FAILED_STEPS + 1))
  fi

  if [[ "${name}" == "fault_recovery_mttr" ]]; then
    MTTR_MS="${duration_ms}"
  fi

  local esc_cmd
  esc_cmd=$(printf '%s' "${cmd[*]}" | sed 's/"/\\"/g')
  STEPS_JSON+="{\"name\":\"${name}\",\"category\":\"${category}\",\"command\":\"${esc_cmd}\",\"passed\":${passed},\"exit_code\":${exit_code},\"duration_ms\":${duration_ms}},"
}

calc_p95() {
  if [[ ${#DURATIONS[@]} -eq 0 ]]; then
    echo "0"
    return
  fi
  mapfile -t sorted < <(printf '%s\n' "${DURATIONS[@]}" | sort -n)
  local count="${#sorted[@]}"
  local idx=$(( (95 * count + 99) / 100 - 1 ))
  if [[ ${idx} -lt 0 ]]; then idx=0; fi
  if [[ ${idx} -ge ${count} ]]; then idx=$((count - 1)); fi
  echo "${sorted[$idx]}"
}

# D4 pressure
run_timed_step "single_session_long_chain" "pressure" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e
run_timed_step "multi_session_concurrency" "pressure" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test p5_perf_stability baseline_concurrent_execute_is_stable
run_timed_step "tool_parallel_mixed_chain" "pressure" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_query_loop_parallel_tool_events_contract

# D5 faults
run_timed_step "fault_provider_timeout" "fault_injection" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_provider_outage_switches_to_degrade_fallback
run_timed_step "fault_tool_failure" "fault_injection" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_mcp_failure_switches_to_conservative_degrade
run_timed_step "fault_budget_overflow_compact_replan" "fault_injection" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib tests::swarm_budget_preflight_compacts_when_budget_overflows
run_timed_step "fault_recovery_mttr" "fault_injection" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib runtime::tests::p11_recover_marks_failover_with_mttr

P95_MS="$(calc_p95)"
ERROR_RATE="$(awk -v f="${FAILED_STEPS}" -v t="${TOTAL_STEPS}" 'BEGIN { if (t == 0) print "0"; else printf "%.4f", f/t }')"
SLO_PASSED="true"
BREACHES=()

if awk -v a="${P95_MS}" -v b="${P95_THRESHOLD_MS}" 'BEGIN { exit !(a > b) }'; then
  BREACHES+=("p95_latency_ms")
  SLO_PASSED="false"
fi
if awk -v a="${ERROR_RATE}" -v b="${ERROR_RATE_THRESHOLD}" 'BEGIN { exit !(a > b) }'; then
  BREACHES+=("error_rate")
  SLO_PASSED="false"
fi
if awk -v a="${MTTR_MS}" -v b="${MTTR_THRESHOLD_MS}" 'BEGIN { exit !(a > b) }'; then
  BREACHES+=("mttr_ms")
  SLO_PASSED="false"
fi

if [[ -n "${STEPS_JSON}" ]]; then
  STEPS_JSON="[${STEPS_JSON%,}]"
else
  STEPS_JSON="[]"
fi

if [[ ${#BREACHES[@]} -gt 0 ]]; then
  BREACH_JSON="$(printf '"%s",' "${BREACHES[@]}")"
  BREACH_JSON="[${BREACH_JSON%,}]"
else
  BREACH_JSON="[]"
fi

ALL_STEPS_PASSED="true"
if [[ ${FAILED_STEPS} -gt 0 ]]; then
  ALL_STEPS_PASSED="false"
fi

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "manifest": "${MANIFEST_PATH}",
  "slo": {
    "p95_latency_ms": ${P95_MS},
    "error_rate": ${ERROR_RATE},
    "mttr_ms": ${MTTR_MS}
  },
  "thresholds": {
    "p95_latency_ms": ${P95_THRESHOLD_MS},
    "error_rate": ${ERROR_RATE_THRESHOLD},
    "mttr_ms": ${MTTR_THRESHOLD_MS}
  },
  "all_steps_passed": ${ALL_STEPS_PASSED},
  "slo_passed": ${SLO_PASSED},
  "breaches": ${BREACH_JSON},
  "steps": ${STEPS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "D46_SLO_LOG=${LOG_PATH}"
echo "D46_SLO_JSON=${JSON_PATH}"

if [[ "${ALL_STEPS_PASSED}" != "true" ]]; then
  echo "D4-D6 acceptance failed: one or more pressure/fault steps failed" >&2
  exit 1
fi
if [[ "${SLO_PASSED}" != "true" ]]; then
  echo "D4-D6 SLO breached: ${BREACHES[*]}" >&2
  exit 1
fi

echo "D46_SLO_ACCEPTANCE_OK"

