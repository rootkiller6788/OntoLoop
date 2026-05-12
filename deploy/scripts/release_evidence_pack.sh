#!/usr/bin/env bash
set -euo pipefail

RUNTIME_DIR="${1:-deploy/runtime}"
PROD_CONFIG_PATH="${2:-deploy/config/autoloop.prod.toml}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

runtime_abs="$(cd "${RUNTIME_DIR}" && pwd)"
evidence_dir="${runtime_abs}/evidence"
mkdir -p "${evidence_dir}"

stamp="$(date +%Y%m%d-%H%M%S)"
ledger_path="${evidence_dir}/release-evidence-${stamp}.json"
latest_path="${evidence_dir}/release-evidence-latest.json"
ledger_chain_path="${evidence_dir}/release-evidence-ledger.jsonl"

commit_sha="$(git -C "${REPO_ROOT}" rev-parse HEAD 2>/dev/null || true)"
branch="$(git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
if [[ -n "$(git -C "${REPO_ROOT}" status --porcelain 2>/dev/null)" ]]; then
  dirty="true"
else
  dirty="false"
fi

json_escape() {
  sed 's/\\/\\\\/g; s/"/\\"/g'
}

file_digest_json() {
  local p="$1"
  if [[ ! -f "${p}" ]]; then
    echo "null"
    return 0
  fi
  local abs size hash mtime
  abs="$(cd "$(dirname "${p}")" && pwd)/$(basename "${p}")"
  size="$(wc -c < "${p}" | tr -d ' ')"
  hash="$(sha256sum "${p}" | awk '{print $1}')"
  mtime="$(date -u -r "${p}" +%Y-%m-%dT%H:%M:%SZ)"
  cat <<EOF
{"path":"$(printf '%s' "${abs}" | json_escape)","size_bytes":${size},"sha256":"${hash}","last_write_time":"${mtime}"}
EOF
}

latest_by_pattern() {
  local pattern="$1"
  ls -1t ${pattern} 2>/dev/null | head -n 1 || true
}

cargo_lock="${REPO_ROOT}/Cargo.lock"
prod_config="${REPO_ROOT}/${PROD_CONFIG_PATH}"
release_gate="${runtime_abs}/release_gate.json"
daily_release="${runtime_abs}/daily_release_package.json"
week6_full="${runtime_abs}/week6_full_acceptance.json"
artifact_proof="$(latest_by_pattern "${runtime_abs}/week6-artifact-proof-*.json")"
d13_latest="$(latest_by_pattern "${runtime_abs}/d13-realbiz-benchmark-*.json")"
d14_latest="$(latest_by_pattern "${runtime_abs}/d14-rollout-*.json")"

script_lines="$(
  find "${REPO_ROOT}/deploy/scripts" -maxdepth 1 -type f \( -name '*.ps1' -o -name '*.sh' \) -print \
    | sort \
    | while read -r f; do
        rel="${f#${REPO_ROOT}/}"
        h="$(sha256sum "${f}" | awk '{print $1}')"
        printf '%s:%s\n' "${rel}" "${h}"
      done
)"
scripts_sha="$(printf '%s' "${script_lines}" | sha256sum | awk '{print $1}')"

collect_refs() {
  local key="$1"
  shift
  grep -h -oE "\"${key}\"[[:space:]]*:[[:space:]]*\"[^\"]+\"" "$@" 2>/dev/null \
    | sed -E "s/.*\"${key}\"[[:space:]]*:[[:space:]]*\"([^\"]+)\".*/\1/" \
    | sort -u
}

core_files=()
for f in "${release_gate}" "${daily_release}" "${week6_full}" "${artifact_proof}" "${d13_latest}" "${d14_latest}"; do
  if [[ -n "${f}" && -f "${f}" ]]; then
    core_files+=("${f}")
  fi
done

evidence_refs="$(collect_refs evidence_ref "${core_files[@]}" | awk '{printf("%s\"%s\"", (NR==1?"":","), $0)}')"
replay_fps="$(collect_refs replay_fp "${core_files[@]}" | awk '{printf("%s\"%s\"", (NR==1?"":","), $0)}')"
wal_tx_ids="$(
  {
    collect_refs wal_tx_id "${core_files[@]}"
    collect_refs wal_id "${core_files[@]}"
  } | sort -u | awk '{printf("%s\"%s\"", (NR==1?"":","), $0)}'
)"

artifact_json="null"
if [[ -n "${artifact_proof}" && -f "${artifact_proof}" ]]; then
  artifact_sha="$(grep -oE '"sha256"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  artifact_path="$(grep -oE '"artifact_path"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  write_proof_ref="$(grep -oE '"write_proof_ref"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  write_proof_hash="$(grep -oE '"write_proof_hash"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  art_evidence_ref="$(grep -oE '"evidence_ref"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  proof_status="$(grep -oE '"proof_status"[[:space:]]*:[[:space:]]*"[^"]+"' "${artifact_proof}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  proof_file_hash="$(sha256sum "${artifact_proof}" | awk '{print $1}')"
  artifact_json="$(cat <<EOF
{"proof_file":"$(printf '%s' "${artifact_proof}" | json_escape)","proof_file_sha256":"${proof_file_hash}","artifact_path":"$(printf '%s' "${artifact_path}" | json_escape)","artifact_sha256":"${artifact_sha}","write_proof_ref":"$(printf '%s' "${write_proof_ref}" | json_escape)","write_proof_hash":"${write_proof_hash}","evidence_ref":"$(printf '%s' "${art_evidence_ref}" | json_escape)","proof_status":"${proof_status}"}
EOF
)"
fi

prev_path=""
prev_sha=""
if [[ -f "${latest_path}" ]]; then
  prev_path="$(grep -oE '"path"[[:space:]]*:[[:space:]]*"[^"]+"' "${latest_path}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  prev_sha="$(grep -oE '"sha256"[[:space:]]*:[[:space:]]*"[^"]+"' "${latest_path}" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/' || true)"
fi

cat > "${ledger_path}" <<EOF
{
  "schema_version": "release-evidence-ledger/v1",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "$(printf '%s' "${REPO_ROOT}" | json_escape)",
  "git": {
    "commit_sha": "${commit_sha}",
    "branch": "$(printf '%s' "${branch}" | json_escape)",
    "dirty": ${dirty}
  },
  "hashes": {
    "cargo_lock": $(file_digest_json "${cargo_lock}"),
    "prod_config": $(file_digest_json "${prod_config}"),
    "scripts_sha256": "${scripts_sha}",
    "release_gate": $(file_digest_json "${release_gate}"),
    "daily_release_package": $(file_digest_json "${daily_release}"),
    "week6_full_acceptance": $(file_digest_json "${week6_full}"),
    "d13_latest": $(file_digest_json "${d13_latest}"),
    "d14_latest": $(file_digest_json "${d14_latest}")
  },
  "artifact": ${artifact_json},
  "refs": {
    "evidence_ref": [${evidence_refs}],
    "replay_fp": [${replay_fps}],
    "wal_tx_id": [${wal_tx_ids}]
  },
  "chain": {
    "previous_ledger_path": "$(printf '%s' "${prev_path}" | json_escape)",
    "previous_ledger_sha256": "${prev_sha}"
  }
}
EOF

ledger_sha="$(sha256sum "${ledger_path}" | awk '{print $1}')"

cat > "${latest_path}" <<EOF
{
  "schema_version": "release-evidence-latest/v1",
  "updated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "current_ledger": {
    "path": "$(printf '%s' "${ledger_path}" | json_escape)",
    "sha256": "${ledger_sha}"
  }
}
EOF

printf '{"generated_at":"%s","ledger_path":"%s","ledger_sha256":"%s","commit_sha":"%s"}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "${ledger_path}" \
  "${ledger_sha}" \
  "${commit_sha}" >> "${ledger_chain_path}"

echo "RELEASE_EVIDENCE_LEDGER_JSON=${ledger_path}"
echo "RELEASE_EVIDENCE_LATEST_JSON=${latest_path}"
echo "RELEASE_EVIDENCE_LEDGER_SHA256=${ledger_sha}"
