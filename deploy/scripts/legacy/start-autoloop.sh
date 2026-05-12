#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
SETTINGS_FILE="${RUNTIME_DIR}/operator-settings.json"
BACKEND_LOG="${RUNTIME_DIR}/backend-live.log"
FRONTEND_LOG="${RUNTIME_DIR}/frontend-live.log"
CONFIG_PATH="${REPO_ROOT}/deploy/config/autoloop.dev.toml"

mkdir -p "${RUNTIME_DIR}"

if [[ -z "${OPENAI_API_KEY:-}" && -f "${SETTINGS_FILE}" ]]; then
  export OPENAI_API_KEY="$(python3 - <<'PY' "${SETTINGS_FILE}"
import json, sys
try:
    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        print(json.load(fh).get("api_key", ""))
except Exception:
    print("")
PY
)"
fi

if [[ -f "${SETTINGS_FILE}" ]]; then
  export AUTOLOOP_PROVIDER_BASE_URL="$(python3 - <<'PY' "${SETTINGS_FILE}"
import json, sys
try:
    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        print(json.load(fh).get("api_base_url", ""))
except Exception:
    print("")
PY
)"
  export AUTOLOOP_PROVIDER_MODEL="$(python3 - <<'PY' "${SETTINGS_FILE}"
import json, sys
try:
    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        print(json.load(fh).get("default_model", ""))
except Exception:
    print("")
PY
)"
  export AUTOLOOP_PROVIDER_VENDOR="$(python3 - <<'PY' "${SETTINGS_FILE}"
import json, sys
try:
    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        print(json.load(fh).get("provider_vendor", ""))
except Exception:
    print("")
PY
)"
fi

if [[ ! -x "${REPO_ROOT}/target/debug/autoloop" ]]; then
  cargo build --manifest-path "${REPO_ROOT}/Cargo.toml"
fi

# startup preflight: config doctor must pass before serving
DOCTOR_RAW="$(cd "${REPO_ROOT}" && cargo run --manifest-path "${REPO_ROOT}/Cargo.toml" -- --config "${CONFIG_PATH}" --session startup-doctor system config doctor --profile local)"
DOCTOR_STATUS="$(DOCTOR_RAW="${DOCTOR_RAW}" python3 - <<'PY'
import json, os
raw = os.environ.get("DOCTOR_RAW", "")
start = raw.find("{")
if start < 0:
    print("parse_error")
else:
    try:
        obj = json.loads(raw[start:])
        print(obj.get("status", "unknown"))
    except Exception:
        print("parse_error")
PY
)"
if [[ "${DOCTOR_STATUS}" != "ok" && "${DOCTOR_STATUS}" != "warn" ]]; then
  echo "startup preflight failed: config doctor status=${DOCTOR_STATUS}" >&2
  exit 1
fi

if [[ ! -d "${REPO_ROOT}/dashboard-ui/node_modules" ]]; then
  (cd "${REPO_ROOT}/dashboard-ui" && npm install)
fi

pkill -f "autoloop.*system serve" || true
pkill -f "vite --host 127.0.0.1 --port 5174" || true

nohup "${REPO_ROOT}/target/debug/autoloop" --config "${CONFIG_PATH}" system serve --host 127.0.0.1 --port 8787 > "${BACKEND_LOG}" 2>&1 &
(cd "${REPO_ROOT}/dashboard-ui" && nohup npx vite --host 127.0.0.1 --port 5174 > "${FRONTEND_LOG}" 2>&1 &)

sleep 6
echo "AutoLoop started."
echo "Dashboard: http://127.0.0.1:5174"
echo "Backend:   http://127.0.0.1:8787/health"
