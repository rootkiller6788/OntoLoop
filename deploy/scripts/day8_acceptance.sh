#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUNTIME_DIR="$REPO_ROOT/deploy/runtime"
mkdir -p "$RUNTIME_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="$RUNTIME_DIR/day8-acceptance-$STAMP.log"
JSON_PATH="$RUNTIME_DIR/day8-acceptance-$STAMP.json"

run_step() {
  local name="$1"
  shift
  echo "==== RUN: [$name] $* ====" | tee -a "$LOG_PATH"
  "$@" 2>&1 | tee -a "$LOG_PATH"
}

pushd "$REPO_ROOT" >/dev/null
run_step cargo-check cargo check --workspace --manifest-path "$MANIFEST_PATH"
run_step e2e-plugin-rollout-hotupdate cargo test --manifest-path "$MANIFEST_PATH" --test pq7_plugin_lifecycle_signed_e2e
run_step lib-query-replay-plugin-trace cargo test --manifest-path "$MANIFEST_PATH" --lib query_plane_aggregates_mismatch_explanations
run_step lib-query-policy-routing cargo test --manifest-path "$MANIFEST_PATH" --lib query_plane_policy_controls_graph_and_routing_surface
popd >/dev/null

cat > "$JSON_PATH" <<EOF
{
  "generated_at": "$(date -Iseconds)",
  "repo_root": "$REPO_ROOT",
  "manifest": "$MANIFEST_PATH",
  "all_passed": true,
  "focus": [
    "plugin shadow/canary/full/quick-rollback lifecycle",
    "query/replay plugin execution trace visibility",
    "mismatch explainer plugin cause surface"
  ],
  "log_path": "$LOG_PATH"
}
EOF

echo "DAY8_ACCEPTANCE_OK log=$LOG_PATH"
echo "DAY8_ACCEPTANCE_JSON=$JSON_PATH"
