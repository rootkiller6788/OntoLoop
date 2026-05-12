#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-d12-ops}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
JSON_PATH="${RUNTIME_DIR}/d12-ops-productized-${STAMP}.json"

mkdir -p "${RUNTIME_DIR}"
cd "${REPO_ROOT}"

OPS_RAW="$(bash ./deploy/scripts/ops_acceptance.sh "${MANIFEST_PATH}" "${PROD_CONFIG_PATH}" "${SESSION_PREFIX}-ops")"
OPS_JSON_PATH="$(printf '%s\n' "${OPS_RAW}" | grep 'OPS_ACCEPTANCE_JSON=' | tail -n1 | cut -d= -f2-)"
if [[ -z "${OPS_JSON_PATH}" || ! -f "${OPS_JSON_PATH}" ]]; then
  echo "failed to locate OPS_ACCEPTANCE_JSON path" >&2
  exit 1
fi

SLO_RAW="$(bash ./deploy/scripts/d46_slo_acceptance.sh "${MANIFEST_PATH}")"
SLO_JSON_PATH="$(printf '%s\n' "${SLO_RAW}" | grep 'D46_SLO_JSON=' | tail -n1 | cut -d= -f2-)"
if [[ -z "${SLO_JSON_PATH}" || ! -f "${SLO_JSON_PATH}" ]]; then
  echo "failed to locate D46_SLO_JSON path" >&2
  exit 1
fi

python3 - <<PY
import json
from pathlib import Path

ops_path = Path(r"${OPS_JSON_PATH}")
slo_path = Path(r"${SLO_JSON_PATH}")
out_path = Path(r"${JSON_PATH}")

ops = json.loads(ops_path.read_text(encoding="utf-8"))
slo = json.loads(slo_path.read_text(encoding="utf-8"))
all_passed = bool(ops.get("all_passed", False)) and bool(slo.get("slo_passed", False))

summary = {
    "generated_at": "${STAMP}",
    "all_passed": all_passed,
    "required_checks": [
        "config-doctor",
        "startup-preflight-config-check",
        "health-check",
        "alert-drill",
        "self-heal-drill",
        "slo-thresholds",
    ],
    "artifacts": {
        "ops_acceptance_json": str(ops_path),
        "slo_acceptance_json": str(slo_path),
    },
    "results": {
        "config_doctor": ops.get("artifacts", {}).get("config_doctor"),
        "health": ops.get("artifacts", {}).get("health"),
        "alert_drill": ops.get("artifacts", {}).get("alert_drill"),
        "self_heal": ops.get("artifacts", {}).get("self_heal"),
        "slo": slo.get("slo"),
        "slo_breaches": slo.get("breaches"),
    },
}

out_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
if not all_passed:
    raise SystemExit("D12 ops productized acceptance failed")
PY

echo "D12_OPS_PRODUCTIZED_OK"
echo "D12_OPS_PRODUCTIZED_JSON=${JSON_PATH}"
