#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=""
SESSION=""
MODE=""
BATCH_NO="1"
TARGETS_JSON="[]"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    --session) SESSION="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --batch-no) BATCH_NO="$2"; shift 2 ;;
    --targets-json) TARGETS_JSON="$2"; shift 2 ;;
    *) shift ;;
  esac
done

REPO_ROOT="${REPO_ROOT:-D:/AutoLoop/autoloop-app}"
SESSION="${SESSION:-sync:pwiki}"
MODE="${MODE:-dry-run}"

if [[ "${MODE}" == "dry-run" ]]; then
  echo "{\"script\":\"pwiki_sync.sh\",\"repo_root\":\"${REPO_ROOT}\",\"session\":\"${SESSION}\",\"mode\":\"${MODE}\",\"batch_no\":${BATCH_NO},\"executed\":false,\"targets_json\":${TARGETS_JSON}}"
  exit 0
fi

MANIFEST_PATH="${REPO_ROOT}/Cargo.toml"
cargo run --manifest-path "${MANIFEST_PATH}" -- --session "${SESSION}" memory patch queue >/tmp/pwiki_sync_step1.log 2>&1 || true
cargo run --manifest-path "${MANIFEST_PATH}" -- --session "${SESSION}" memory compiler status --repo-root "${REPO_ROOT}" >/tmp/pwiki_sync_step2.log 2>&1 || true
cargo run --manifest-path "${MANIFEST_PATH}" -- --session "${SESSION}" memory graph export --repo-root "${REPO_ROOT}" --clean --report >/tmp/pwiki_sync_step3.log 2>&1 || true

echo "{\"script\":\"pwiki_sync.sh\",\"repo_root\":\"${REPO_ROOT}\",\"session\":\"${SESSION}\",\"mode\":\"${MODE}\",\"batch_no\":${BATCH_NO},\"executed\":true,\"targets_json\":${TARGETS_JSON},\"steps\":[\"memory_patch_queue\",\"memory_compiler_status\",\"memory_graph_export\"]}"
