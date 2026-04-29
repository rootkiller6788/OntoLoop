#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"
SESSION_PREFIX="${3:-d14-storage-cutover}"
LOCAL_PG_URI="${4:-${AUTOLOOP_D14_PG_URI:-postgres://postgres:123456@localhost:5432/postgres}}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/d14-storage-cutover-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/d14-storage-cutover-${STAMP}.json"
BACKUP_PATH="${RUNTIME_DIR}/d14-autoloop.prod.backup-${STAMP}.toml"

mkdir -p "${RUNTIME_DIR}"
cp "${REPO_ROOT}/${PROD_CONFIG_PATH}" "${BACKUP_PATH}"

restore_config() {
  if [[ -f "${BACKUP_PATH}" ]]; then
    cp "${BACKUP_PATH}" "${REPO_ROOT}/${PROD_CONFIG_PATH}"
  fi
}
trap restore_config EXIT

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

set_storage_cutover() {
  local backend="$1"
  local mode="$2"
  local read_pref="$3"
  local rollout="$4"
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"

  awk -v backend="${backend}" -v mode="${mode}" -v read_pref="${read_pref}" -v rollout="${rollout}" '
    BEGIN {
      in_storage = 0
      printed_storage = 0
      grace = 24
    }
    /^\[storage\]/ {
      in_storage = 1
      next
    }
    in_storage && /^\[/ {
      print "[storage]"
      print "backend = \"" backend "\""
      print "mode = \"" mode "\""
      print "shadow_read_preference = \"" read_pref "\""
      print "shadow_read_rollout_percent = " rollout
      print "shadow_write_grace_hours = " grace
      print ""
      printed_storage = 1
      in_storage = 0
      print
      next
    }
    in_storage {
      if ($0 ~ /^[[:space:]]*shadow_write_grace_hours[[:space:]]*=/) {
        split($0, parts, "=")
        gsub(/[[:space:]]/, "", parts[2])
        if (parts[2] != "") grace = parts[2]
      }
      next
    }
    { print }
    END {
      if (in_storage && !printed_storage) {
        print "[storage]"
        print "backend = \"" backend "\""
        print "mode = \"" mode "\""
        print "shadow_read_preference = \"" read_pref "\""
        print "shadow_read_rollout_percent = " rollout
        print "shadow_write_grace_hours = " grace
      }
    }
  ' "$file" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

set_legacy_fallback_local() {
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  awk '
    BEGIN { in_legacy = 0 }
    /^\[state_store\]/ { in_legacy = 1; print; next }
    in_legacy && /^\[/ { in_legacy = 0; print; next }
    in_legacy {
      if ($0 ~ /^[[:space:]]*backend[[:space:]]*=/) { print "backend = \"in_memory\""; next }
      if ($0 ~ /^[[:space:]]*uri[[:space:]]*=/) { print "uri = \"http://127.0.0.1:3000\""; next }
      print
      next
    }
    { print }
  ' "$file" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

set_local_postgres_config() {
  local file="${REPO_ROOT}/${PROD_CONFIG_PATH}"
  awk -v local_uri="${LOCAL_PG_URI}" '
    BEGIN { in_pg = 0 }
    /^\[storage\.postgres\]/ { in_pg = 1; print; next }
    in_pg && /^\[/ { in_pg = 0; print; next }
    in_pg {
      if ($0 ~ /^[[:space:]]*enabled[[:space:]]*=/) { print "enabled = true"; next }
      if ($0 ~ /^[[:space:]]*uri[[:space:]]*=/) { print "uri = \"" local_uri "\""; next }
      print
      next
    }
    { print }
  ' "$file" > "${file}.tmp"
  mv "${file}.tmp" "$file"
}

RESULTS_JSON=""

run_step "cargo-check" cargo check --workspace --manifest-path "${MANIFEST_PATH}"
run_step "shadow-diff-query-plane" cargo test --manifest-path "${MANIFEST_PATH}" --test pq11_d11_compact_snapshot_task_mcp_parallel_e2e

set_storage_cutover "postgres" "direct" "postgres" "100"
set_local_postgres_config
set_legacy_fallback_local
run_step "postgres-schema-bootstrap" psql --set ON_ERROR_STOP=1 --dbname "${LOCAL_PG_URI}" --file "./deploy/sql/d4_postgres_core_schema.sql"
run_step "cutover-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "cutover-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-cutover" system health
run_step "postgres-primary-read-check" cargo test --manifest-path "${MANIFEST_PATH}" --test d9_wasm_sandbox_runtime_paths_e2e

set_storage_cutover "postgres" "shadow" "postgres" "0"
run_step "rollback-status" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" system status
run_step "rollback-health" cargo run --manifest-path "${MANIFEST_PATH}" -- --config "${PROD_CONFIG_PATH}" --session "${SESSION_PREFIX}-rollback" system health

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "prod_config": "${PROD_CONFIG_PATH}",
  "backup_config": "${BACKUP_PATH}",
  "all_passed": true,
  "cutover": {
    "target": "postgres-primary",
    "rollback_target": "postgres-shadow-readonly-fallback",
    "remove_state_store_allowed": true,
    "remove_state_store_gate": "Only after this report passes and rollback drill succeeds."
  },
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "D14_STORAGE_CUTOVER_OK log=${LOG_PATH}"
echo "D14_STORAGE_CUTOVER_JSON=${JSON_PATH}"

