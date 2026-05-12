#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-d14-rollout}"
DRY_RUN="${4:-false}"
PROFILE="${AUTOLOOP_PROFILE:-production-e2e}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/d14-rollout-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/d14-rollout-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/d14-rollout-config-backup-${STAMP}.toml"

mkdir -p "${RUNTIME_DIR}"
cp "${REPO_ROOT}/${PROD_CONFIG_PATH}" "${BACKUP_PATH}"

RESULTS_JSON=""
ALL_PASSED=true
FAILURE=""
ROLLBACK_TRIGGERED=false
ROLLBACK_REASON=""
ROLLBACK_STATUS_OK=false
ROLLBACK_HEALTH_OK=false

restore_config() {
  if [[ -f "${BACKUP_PATH}" ]]; then
    cp "${BACKUP_PATH}" "${REPO_ROOT}/${PROD_CONFIG_PATH}"
  fi
}
trap restore_config EXIT
export AUTOLOOP_PROFILE="${PROFILE}"

run_step() {
  local name="$1"
  shift
  local cmd=("$@")
  local printable
  printable=$(printf '%q ' "${cmd[@]}")
  echo "" >> "${LOG_PATH}"
  echo "==== RUN: [${name}] ${printable}====" >> "${LOG_PATH}"

  if [[ "${DRY_RUN}" == "true" ]]; then
    echo "DRY_RUN=true (skipped execution)" >> "${LOG_PATH}"
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${printable}\",\"passed\":true,\"skipped\":true,\"exit_code\":0},"
    return 0
  fi

  if (
    cd "${REPO_ROOT}"
    "${cmd[@]}"
  ) >> "${LOG_PATH}" 2>&1; then
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${printable}\",\"passed\":true,\"skipped\":false,\"exit_code\":0},"
    return 0
  else
    local exit_code=$?
    RESULTS_JSON+="{\"name\":\"${name}\",\"command\":\"${printable}\",\"passed\":false,\"skipped\":false,\"exit_code\":${exit_code}},"
    return ${exit_code}
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

normalize_config_for_local_rollout() {
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  awk '
    /^\[.*\]$/ {
      section = $0
      print
      next
    }
    section == "[state_store]" && $0 ~ /^[[:space:]]*backend[[:space:]]*=/ {
      print "backend = \"in_memory\""
      next
    }
    section == "[storage]" && $0 ~ /^[[:space:]]*backend[[:space:]]*=/ {
      print "backend = \"postgres\""
      next
    }
    section == "[storage]" && $0 ~ /^[[:space:]]*shadow_read_preference[[:space:]]*=/ {
      print "shadow_read_preference = \"postgres\""
      next
    }
    section == "[storage.postgres]" && $0 ~ /^[[:space:]]*enabled[[:space:]]*=/ {
      print "enabled = true"
      next
    }
    section == "[storage.postgres]" && $0 ~ /^[[:space:]]*uri[[:space:]]*=/ {
      print "uri = \"postgres://postgres:123456@localhost:5432/ontoloop\""
      next
    }
    {
      print
    }
  ' "$file" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

run_config_doctor_gate() {
  local session_id="$1"
  local profile="$2"
  local out="${RUNTIME_DIR}/d14-config-doctor-${STAMP}.json"
  run_step "pre-rollout-config-doctor-gate" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${session_id}" system config doctor --profile "${profile}" --output "${out}"
  if [[ ! -f "${out}" ]]; then
    echo "config doctor output missing: ${out}" >> "${LOG_PATH}"
    return 1
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
}

run_rollback() {
  local reason="$1"
  ROLLBACK_TRIGGERED=true
  ROLLBACK_REASON="$reason"
  set_gate_config "shadow" "0.2"

  if run_step "rollback-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status; then
    ROLLBACK_STATUS_OK=true
  fi
  if run_step "rollback-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-rollback" system health; then
    ROLLBACK_HEALTH_OK=true
  fi
}

if ! run_step "cargo-check" cargo check --workspace --manifest-path "${MANIFEST_PATH}"; then
  ALL_PASSED=false
  FAILURE="cargo-check failed"
fi

if [[ "${ALL_PASSED}" == "true" ]]; then
  if ! run_step "rollout-gating-test" cargo test --manifest-path "${MANIFEST_PATH}" --test p6_rollout_gating; then
    ALL_PASSED=false
    FAILURE="rollout-gating-test failed"
  fi
fi

if [[ "${ALL_PASSED}" == "true" ]]; then
  normalize_config_for_local_rollout
fi

if [[ "${ALL_PASSED}" == "true" ]]; then
  if ! run_config_doctor_gate "${SESSION_PREFIX}-config-doctor" "${PROFILE}"; then
    ALL_PASSED=false
    FAILURE="pre-rollout-config-doctor-gate failed"
  fi
fi

if [[ "${ALL_PASSED}" == "true" ]]; then
  declare -a STAGES=("shadow:shadow:0.2" "canary10:canary:0.1" "canary30:canary:0.3" "full:full:1.0")
  for stage_spec in "${STAGES[@]}"; do
    IFS=":" read -r stage_name mode ratio <<< "${stage_spec}"
    set_gate_config "${mode}" "${ratio}"
    if ! run_step "rollout-${stage_name}-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status; then
      ALL_PASSED=false
      FAILURE="rollout-${stage_name}-status failed"
      break
    fi
    if ! run_step "rollout-${stage_name}-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-${stage_name}" system health; then
      ALL_PASSED=false
      FAILURE="rollout-${stage_name}-health failed"
      break
    fi
  done
fi

if [[ "${ALL_PASSED}" == "true" ]]; then
  run_rollback "post_full_drill"
else
  echo "" >> "${LOG_PATH}"
  echo "==== FAILURE ==== ${FAILURE}" >> "${LOG_PATH}"
  run_rollback "auto_rollback_on_failure"
fi

RESULTS_JSON="[${RESULTS_JSON%,}]"
cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "backup_config": "${BACKUP_PATH}",
  "all_passed": ${ALL_PASSED},
  "dry_run": ${DRY_RUN},
  "rollout": ["shadow", "10%", "30%", "full", "rollback"],
  "failure": "${FAILURE}",
  "rollback": {
    "triggered": ${ROLLBACK_TRIGGERED},
    "reason": "${ROLLBACK_REASON}",
    "status_ok": ${ROLLBACK_STATUS_OK},
    "health_ok": ${ROLLBACK_HEALTH_OK}
  },
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

if [[ "${ALL_PASSED}" == "true" ]]; then
  echo "D14_ROLLOUT_OK log=${LOG_PATH}"
  echo "D14_ROLLOUT_JSON=${JSON_PATH}"
  exit 0
else
  echo "D14_ROLLOUT_FAILED log=${LOG_PATH}"
  echo "D14_ROLLOUT_JSON=${JSON_PATH}"
  exit 1
fi
