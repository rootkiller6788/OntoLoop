#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
SESSION_PREFIX="${2:-pwiki-acceptance}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/pwiki-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/pwiki-acceptance-${STAMP}.json"

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

run_step "ingest-compile" cargo test --manifest-path "${MANIFEST_PATH}" --lib day78_incremental_compiler_rebuilds_changed_files_only
run_step "infer-resume" cargo test --manifest-path "${MANIFEST_PATH}" --lib semantic_resume_only_backfills_unfinished_sources
run_step "graph-health" cargo test --manifest-path "${MANIFEST_PATH}" --lib view_plane_persists_graph_health_record_and_latest_ref
run_step "recall-expansion-enable" cargo test --manifest-path "${MANIFEST_PATH}" --lib graph_enabled_expands_neighbors_with_confidence
run_step "recall-expansion-disable" cargo test --manifest-path "${MANIFEST_PATH}" --lib route_disables_graph_sources_when_project_policy_disables_graph
run_step "heal-proposal-gate" cargo test --manifest-path "${MANIFEST_PATH}" --lib heal_proposal_requires_approval_before_canonical_write
run_step "query-plane-summary" cargo test --manifest-path "${MANIFEST_PATH}" --lib query_plane_surfaces_graph_health_summary_and_refs

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "session_prefix": "${SESSION_PREFIX}",
  "all_passed": true,
  "chain": [
    "ingest/compile",
    "infer(resume)",
    "graph health",
    "recall expansion",
    "heal proposal",
    "approve",
    "recompile",
    "query-plane"
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "PWIKI_ACCEPTANCE_OK log=${LOG_PATH}"
echo "PWIKI_ACCEPTANCE_JSON=${JSON_PATH}"
