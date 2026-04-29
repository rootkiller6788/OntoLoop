#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
SESSION_PREFIX="${2:-signal-acceptance}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/signal-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/signal-acceptance-${STAMP}.json"

mkdir -p "${RUNTIME_DIR}"
RESULTS_JSON=""

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

run_step "signal-contract-order-reject-replay-e2e" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_signal_pipeline_contract_order_reject_replay_e2e
run_step "signal-no-bypass-static-scan" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib observability::signal_facade::tests::signal_write_path_is_no_bypass
run_step "signal-cli-whitebox-command-surface" \
  cargo test --manifest-path "${MANIFEST_PATH}" --bin ontoloop system_signal_status_and_explain_views_are_available

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "session_prefix": "${SESSION_PREFIX}",
  "all_passed": true,
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "SIGNAL_ACCEPTANCE_OK log=${LOG_PATH}"
echo "SIGNAL_ACCEPTANCE_JSON=${JSON_PATH}"
