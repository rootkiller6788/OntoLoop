#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-./deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-ops-acceptance}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
mkdir -p "${RUNTIME_DIR}"

STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/ops-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/ops-acceptance-${STAMP}.json"

RESULTS_JSON=()

run_step() {
  local name="$1"
  shift
  local cmd=("$@")
  {
    echo
    echo "==== RUN: [${name}] ${cmd[*]} ===="
  } >> "${LOG_PATH}"
  "${cmd[@]}" >> "${LOG_PATH}" 2>&1
  RESULTS_JSON+=("{\"name\":\"${name}\",\"command\":\"$(printf '%s ' "${cmd[@]}" | sed 's/ $//')\",\"passed\":true,\"exit_code\":0}")
}

run_system_json_step() {
  local name="$1"
  local session_id="$2"
  shift 2
  local out="${RUNTIME_DIR}/${name}-${STAMP}.json"
  run_step "${name}" cargo run --manifest-path "${MANIFEST_PATH}" -- \
    --config "${PROD_CONFIG_PATH}" \
    --session "${session_id}" \
    system "$@" --output "${out}"
  if [[ ! -f "${out}" ]]; then
    echo "Expected output file missing: ${out}" >&2
    exit 1
  fi
  echo "${out}"
}

CONFIG_DOCTOR_OUT="$(run_system_json_step "ops-config-doctor" "${SESSION_PREFIX}-doctor" config doctor --profile production)"
HEALTH_OUT="$(run_system_json_step "ops-health-check" "${SESSION_PREFIX}-health" health)"
ALERT_STATUS_OUT="$(run_system_json_step "ops-alert-status" "${SESSION_PREFIX}-alert" alert status)"
ALERT_DRILL_OUT="$(run_system_json_step "ops-alert-drill" "${SESSION_PREFIX}-alert" alert drill --reason "ops drill synthetic alert")"
SELF_HEAL_OUT="$(run_system_json_step "ops-self-heal-drill" "${SESSION_PREFIX}-heal" self-heal drill --profile queue_throttle --reason "ops drill self-heal")"

DOCTOR_STATUS="$(python -c "import json;print(json.load(open('${CONFIG_DOCTOR_OUT}','r',encoding='utf-8')).get('status',''))")"
if [[ "${DOCTOR_STATUS}" != "pass" ]]; then
  echo "config doctor hard gate failed: status=${DOCTOR_STATUS}" >&2
  exit 1
fi

python - <<PY
import json
from pathlib import Path
doctor = json.loads(Path(r"${CONFIG_DOCTOR_OUT}").read_text(encoding="utf-8"))
required = {
    "profile.alignment",
    "runtime.gate_mode",
    "runtime.rollback_window",
    "storage.backend_consistency",
}
checks = {item.get("id"): item for item in doctor.get("checks", []) if isinstance(item, dict)}
missing = [item for item in required if item not in checks]
if missing:
    raise SystemExit(f"config doctor hard gate missing required checks: {missing}")
failed = [item for item in required if not bool(checks[item].get("passed", False))]
if failed:
    raise SystemExit(f"config doctor hard gate checks failed: {failed}")
PY
if [[ $? -ne 0 ]]; then
  echo "config doctor required checks validation failed" >&2
  exit 1
fi

ALERT_DRILL_STATUS="$(python -c "import json;print(json.load(open('${ALERT_DRILL_OUT}','r',encoding='utf-8')).get('status',''))")"
if [[ "${ALERT_DRILL_STATUS}" != "raised" ]]; then
  echo "alert drill did not raise alert" >&2
  exit 1
fi

SELF_HEAL_RECOVERED="$(python -c "import json;print(str(bool(json.load(open('${SELF_HEAL_OUT}','r',encoding='utf-8')).get('recover',{}).get('recovered',False))).lower())")"
if [[ "${SELF_HEAL_RECOVERED}" != "true" ]]; then
  echo "self-heal drill did not recover" >&2
  exit 1
fi

RESULTS_JOINED="$(IFS=,; echo "${RESULTS_JSON[*]}")"
cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -Iseconds)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "all_passed": true,
  "required_checks": ["config-doctor","health-check","alert-status","alert-drill","self-heal-drill"],
  "commands": [${RESULTS_JOINED}],
  "artifacts": {
    "config_doctor": "${CONFIG_DOCTOR_OUT}",
    "health": "${HEALTH_OUT}",
    "alert_status": "${ALERT_STATUS_OUT}",
    "alert_drill": "${ALERT_DRILL_OUT}",
    "self_heal": "${SELF_HEAL_OUT}"
  },
  "log_path": "${LOG_PATH}"
}
EOF

echo "OPS_ACCEPTANCE_OK log=${LOG_PATH}"
echo "OPS_ACCEPTANCE_JSON=${JSON_PATH}"
