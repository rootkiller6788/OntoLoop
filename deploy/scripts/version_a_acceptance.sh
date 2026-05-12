#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
JSON_PATH="${RUNTIME_DIR}/version-a-acceptance.json"

mkdir -p "${RUNTIME_DIR}"
cd "${REPO_ROOT}"

declare -A CASE_RESULT
declare -a CASES
declare -a CASE_CMDS

run_case() {
  local name="$1"
  local cmd="$2"
  CASES+=("${name}")
  CASE_CMDS+=("${cmd}")
  if bash -lc "${cmd}" >/dev/null 2>&1; then
    CASE_RESULT["${name}"]="true"
  else
    CASE_RESULT["${name}"]="false"
  fi
}

run_case "constraint_shield_pass" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib constraint_patterns_block_unsafe_payload"
run_case "task_tree_valid" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib e2r_gate_requires_dependencies_accepted_before_commit"
run_case "ranking_route_valid" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib ranking_is_stable_and_prefers_higher_score"
run_case "bandit_update_valid" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib updates_alpha_beta_posterior_counts"
run_case "completed_not_accepted_without_review" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib e2r_gate_enforces_rejected_iterate_and_accept_commit"
run_case "evidence_commit_valid" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib relation_writes_are_restricted_to_relation_facade"
run_case "wal_atomic_valid" "cargo test --manifest-path \"${REPO_ROOT}/autoloop-postgres-adapter/Cargo.toml\" atomic_relation_bundle_rolls_back_on_failpoint"
run_case "replay_smoke_pass" "cargo test --manifest-path \"${MANIFEST_PATH}\" --lib wal_tx_envelope_roundtrip_is_stable"

# Use bash to produce a stable JSON payload without jq dependency.
all_passed=true
tests_json="["
for i in "${!CASES[@]}"; do
  name="${CASES[$i]}"
  cmd="${CASE_CMDS[$i]}"
  passed="${CASE_RESULT[$name]}"
  if [[ "${passed}" != "true" ]]; then
    all_passed=false
  fi
  tests_json+="{\"name\":\"${name}\",\"passed\":${passed},\"command\":\"${cmd//\"/\\\"}\"}"
  if [[ "$i" -lt "$((${#CASES[@]} - 1))" ]]; then
    tests_json+=","
  fi
done
tests_json+="]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "version": "version-a/v1",
  "all_passed": ${all_passed},
  "constraint_shield_pass": ${CASE_RESULT[constraint_shield_pass]},
  "task_tree_valid": ${CASE_RESULT[task_tree_valid]},
  "ranking_route_valid": ${CASE_RESULT[ranking_route_valid]},
  "bandit_update_valid": ${CASE_RESULT[bandit_update_valid]},
  "completed_not_accepted_without_review": ${CASE_RESULT[completed_not_accepted_without_review]},
  "evidence_commit_valid": ${CASE_RESULT[evidence_commit_valid]},
  "wal_atomic_valid": ${CASE_RESULT[wal_atomic_valid]},
  "replay_smoke_pass": ${CASE_RESULT[replay_smoke_pass]},
  "tests": ${tests_json}
}
EOF

echo "VERSION_A_ACCEPTANCE_JSON=${JSON_PATH}"
if [[ "${all_passed}" != "true" ]]; then
  echo "version-a acceptance failed" >&2
  exit 1
fi
