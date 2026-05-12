#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/day11-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/day11-acceptance-${STAMP}.json"

mkdir -p "${RUNTIME_DIR}"

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

RESULTS_JSON=""

run_step "cargo-check" cargo check --manifest-path "${MANIFEST_PATH}"
run_step "parallel-tool-call-events" cargo test --manifest-path "${MANIFEST_PATH}" --test pq10_query_loop_parallel_tool_events_contract
run_step "two-stage-compact" cargo test --manifest-path "${MANIFEST_PATH}" --test pq2_compaction_resume_boundary
run_step "named-snapshot-transcript" cargo test --manifest-path "${MANIFEST_PATH}" --test pq7_session_named_snapshot_transcript_e2e
run_step "background-task-manager" cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_background_task_manager_e2e
run_step "mcp-manager-service-spine" cargo test --manifest-path "${MANIFEST_PATH}" --test pq8_service_mediation_spine
run_step "d11-aggregate-e2e" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_d11_compact_snapshot_task_mcp_parallel_e2e

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "all_passed": true,
  "required_checks": [
    "parallel_tool_call",
    "two_stage_compact",
    "named_snapshot",
    "task_manager",
    "mcp_manager"
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "DAY11_ACCEPTANCE_OK log=${LOG_PATH}"
echo "DAY11_ACCEPTANCE_JSON=${JSON_PATH}"
