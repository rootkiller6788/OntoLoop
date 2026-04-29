#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
SESSION_PREFIX="${2:-frontend-cli-acceptance}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/frontend-cli-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/frontend-cli-acceptance-${STAMP}.json"

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

run_step "frontend-chat-stream-tool-permission-attach-e2e" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pq12_frontend_cli_chat_stream_tool_permission_attach_e2e
run_step "transport-session-event-contract-v2" \
  cargo test --manifest-path "${MANIFEST_PATH}" --test pq6_transport_session_event_contract_v2
run_step "query-plane-cli-event-chain-visible" \
  cargo test --manifest-path "${MANIFEST_PATH}" --lib observability::query_plane::tests::query_plane_includes_cli_event_chain_records

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

echo "FRONTEND_CLI_ACCEPTANCE_OK log=${LOG_PATH}"
echo "FRONTEND_CLI_ACCEPTANCE_JSON=${JSON_PATH}"
