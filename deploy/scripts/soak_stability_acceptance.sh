#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-soak-stability}"
DURATION_HOURS="${4:-6}"
CASE_TIMEOUT_SEC="${5:-90}"
MAX_RETRIES_PER_CASE="${6:-1}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/soak-stability-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/soak-stability-${STAMP}.json"

mkdir -p "${RUNTIME_DIR}"

if (( DURATION_HOURS < 1 )); then DURATION_HOURS=1; fi
if (( DURATION_HOURS > 8 )); then DURATION_HOURS=8; fi
END_EPOCH=$(( $(date +%s) + DURATION_HOURS * 3600 ))

TOTAL=0
FAILURES=0
RECOVERIES=0
DURATIONS=()
STEPS_JSON=""

run_case_attempt() {
  local session_id="$1"
  local attempt="$2"
  local prompt
  prompt="生成一个可落盘的 HTML 业务页面到 deploy/runtime/soak-artifact-${session_id}.html，必须工具执行并输出可验证结果。"
  local start_ms end_ms duration_ms exit_code
  start_ms="$(date +%s%3N)"
  set +e
  (
    cd "${REPO_ROOT}"
    cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" --message "${prompt}"
  ) >> "${LOG_PATH}" 2>&1
  exit_code=$?
  set -e
  end_ms="$(date +%s%3N)"
  duration_ms=$((end_ms - start_ms))
  printf '{"attempt":%s,"passed":%s,"exit_code":%s,"duration_ms":%s}' \
    "${attempt}" \
    "$([[ ${exit_code} -eq 0 ]] && echo true || echo false)" \
    "${exit_code}" \
    "${duration_ms}"
  return "${exit_code}"
}

while (( $(date +%s) < END_EPOCH )); do
  TOTAL=$((TOTAL + 1))
  session_id="${SESSION_PREFIX}-${STAMP}-${TOTAL}"
  case_passed=false
  attempts_json=""
  success_duration=""

  attempt=0
  while (( attempt <= MAX_RETRIES_PER_CASE )); do
    attempt=$((attempt + 1))
    out_json="$(run_case_attempt "${session_id}" "${attempt}")" || true
    exit_code="$(printf '%s' "${out_json}" | sed -n 's/.*"exit_code":\([0-9]\+\).*/\1/p')"
    duration_ms="$(printf '%s' "${out_json}" | sed -n 's/.*"duration_ms":\([0-9]\+\).*/\1/p')"
    attempts_json+="${out_json},"
    if [[ "${exit_code}" == "0" ]]; then
      case_passed=true
      success_duration="${duration_ms}"
      if (( attempt > 1 )); then RECOVERIES=$((RECOVERIES + 1)); fi
      break
    fi
    if (( attempt <= MAX_RETRIES_PER_CASE )); then
      sleep_for=$(( CASE_TIMEOUT_SEC / 30 ))
      if (( sleep_for < 1 )); then sleep_for=1; fi
      if (( sleep_for > 5 )); then sleep_for=5; fi
      sleep "${sleep_for}"
    fi
  done

  if [[ "${case_passed}" != "true" ]]; then
    FAILURES=$((FAILURES + 1))
  else
    DURATIONS+=("${success_duration}")
  fi

  if [[ -n "${attempts_json}" ]]; then
    attempts_json="[${attempts_json%,}]"
  else
    attempts_json="[]"
  fi
  STEPS_JSON+="{\"case_id\":\"${session_id}\",\"passed\":${case_passed},\"attempts\":${attempts_json}},"
done

p95=0
if (( ${#DURATIONS[@]} > 0 )); then
  mapfile -t sorted < <(printf '%s\n' "${DURATIONS[@]}" | sort -n)
  count=${#sorted[@]}
  idx=$(( (95 * count + 99) / 100 - 1 ))
  if (( idx < 0 )); then idx=0; fi
  if (( idx >= count )); then idx=$((count - 1)); fi
  p95="${sorted[$idx]}"
fi

error_rate="1.0000"
if (( TOTAL > 0 )); then
  error_rate="$(awk -v f="${FAILURES}" -v t="${TOTAL}" 'BEGIN { printf "%.4f", f/t }')"
fi

if [[ -n "${STEPS_JSON}" ]]; then
  STEPS_JSON="[${STEPS_JSON%,}]"
else
  STEPS_JSON="[]"
fi

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "duration_hours": ${DURATION_HOURS},
  "case_timeout_sec": ${CASE_TIMEOUT_SEC},
  "max_retries_per_case": ${MAX_RETRIES_PER_CASE},
  "totals": {
    "cases": ${TOTAL},
    "failures": ${FAILURES},
    "recoveries": ${RECOVERIES}
  },
  "stability": {
    "p95_latency_ms": ${p95},
    "error_rate": ${error_rate},
    "recovery_count": ${RECOVERIES}
  },
  "steps": ${STEPS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "SOAK_STABILITY_LOG=${LOG_PATH}"
echo "SOAK_STABILITY_JSON=${JSON_PATH}"
