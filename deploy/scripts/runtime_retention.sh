#!/usr/bin/env bash
set -euo pipefail

ACTION="${1:-all}"
RETENTION_DAYS="${RETENTION_DAYS:-7}"
BENCHMARK_KEEP_LATEST="${BENCHMARK_KEEP_LATEST:-8}"
APPLY="${APPLY:-0}"
PERSIST_REPORT="${PERSIST_REPORT:-0}"

if [[ "${ACTION}" != "clean-cache" && "${ACTION}" != "archive-evidence" && "${ACTION}" != "clean-old-runtime" && "${ACTION}" != "all" ]]; then
  echo "unsupported action: ${ACTION}" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
RETENTION_DIR="${RUNTIME_DIR}/retention"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT_PATH="${RETENTION_DIR}/runtime-retention-report-${STAMP}.json"

mkdir -p "${RUNTIME_DIR}"

deleted_json="[]"
kept_json="[]"
summary_json="[]"
errors_json="[]"

append_json_array() {
  local base="$1"
  local item="$2"
  jq --argjson item "${item}" '. + [$item]' <<<"${base}"
}

prune_latest_by_pattern() {
  local pattern="$1"
  local keep_latest="$2"
  local type="$3"
  local matched=()
  while IFS= read -r -d '' f; do
    matched+=("${f}")
  done < <(find "${RUNTIME_DIR}" -maxdepth 1 -type f -name "${pattern}" -print0 2>/dev/null || true)

  if (( ${#matched[@]} <= keep_latest )); then
    return 0
  fi

  IFS=$'\n' matched=($(for f in "${matched[@]}"; do printf '%s\n' "${f}"; done | xargs -I{} stat -c '%Y|%n' "{}" | sort -rn | cut -d'|' -f2-))
  unset IFS
  for (( i=keep_latest; i<${#matched[@]}; i++ )); do
    f="${matched[$i]}"
    if [[ -e "${f}" ]]; then
      ts="$(date -u -r "${f}" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
      safe_rm "${f}"
      extra="$(jq -cn --arg t "${ts}" '{last_write_time:$t}')"
      record_delete "${type}" "${f}" "${extra}"
    fi
  done
}

record_delete() {
  local type="$1"
  local path="$2"
  local extra="${3:-{}}"
  local item
  item="$(jq -cn --arg t "${type}" --arg p "${path}" --argjson e "${extra}" '{type:$t,path:$p} + $e')"
  deleted_json="$(append_json_array "${deleted_json}" "${item}")"
}

safe_rm() {
  local target="$1"
  if [[ ! -e "${target}" ]]; then
    return 0
  fi
  if [[ "${APPLY}" == "1" ]]; then
    rm -rf -- "${target}"
  fi
}

if [[ "${ACTION}" == "clean-cache" || "${ACTION}" == "all" ]]; then
  for d in "${REPO_ROOT}/target" "${REPO_ROOT}/autoloop-app/target"; do
    if [[ -e "${d}" ]]; then
      safe_rm "${d}"
      record_delete "cache_dir" "${d}"
    fi
  done

  while IFS= read -r -d '' d; do
    safe_rm "${d}"
    record_delete "runtime_target" "${d}"
  done < <(find "${RUNTIME_DIR}" -maxdepth 1 -type d -name 'target-*' -print0 2>/dev/null || true)

  for d in \
    "${RUNTIME_DIR}/target-final-check" \
    "${RUNTIME_DIR}/target-benchmark-shared" \
    "${RUNTIME_DIR}/target-benchmark-compare"; do
    if [[ -e "${d}" ]]; then
      safe_rm "${d}"
      record_delete "runtime_target_named" "${d}"
    fi
  done

  while IFS= read -r -d '' d; do
    safe_rm "${d}"
    record_delete "incremental_dir" "${d}"
  done < <(find "${REPO_ROOT}/autoloop-app" -type d -name incremental -print0 2>/dev/null || true)

  while IFS= read -r -d '' f; do
    safe_rm "${f}"
    record_delete "pdb_file" "${f}"
  done < <(find "${REPO_ROOT}/autoloop-app" -type f -name '*.pdb' -print0 2>/dev/null || true)

  temp_root="${TMPDIR:-/tmp}"
  if [[ -n "${temp_root}" && -d "${temp_root}" ]]; then
    while IFS= read -r -d '' d; do
      safe_rm "${d}"
      record_delete "temp_target" "${d}"
    done < <(find "${temp_root}" -maxdepth 1 -type d -name 'autoloop-target-*' -print0 2>/dev/null || true)
  fi
fi

if [[ "${ACTION}" == "archive-evidence" || "${ACTION}" == "all" ]]; then
  declare -A keep_map

  for base in release_gate.json daily_release_package.json proof_ledger.jsonl; do
    p="${RUNTIME_DIR}/${base}"
    if [[ -f "${p}" ]]; then
      keep_map["${p}"]=1
    fi
  done

  for p in "${!keep_map[@]}"; do
    hash="$(sha256sum "${p}" | awk '{print $1}')"
    size="$(wc -c < "${p}" | tr -d ' ')"
    mtime="$(date -u -r "${p}" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)"
    item="$(jq -cn --arg name "$(basename "${p}")" --arg path "${p}" --arg sha "${hash}" --arg mt "${mtime}" --argjson size "${size}" \
      '{name:$name,path:$path,size_bytes:$size,sha256:$sha,last_write_time:$mt}')"
    kept_json="$(append_json_array "${kept_json}" "${item}")"
  done

  while IFS= read -r -d '' f; do
    if [[ -n "${keep_map["${f}"]+x}" ]]; then
      continue
    fi
    safe_rm "${f}"
    record_delete "runtime_non_release_file" "${f}"
  done < <(find "${RUNTIME_DIR}" -maxdepth 1 -type f -print0 2>/dev/null || true)

  while IFS= read -r -d '' d; do
    safe_rm "${d}"
    record_delete "runtime_subdir" "${d}"
  done < <(find "${RUNTIME_DIR}" -mindepth 1 -maxdepth 1 -type d -print0 2>/dev/null || true)
fi

if [[ "${ACTION}" == "clean-old-runtime" || "${ACTION}" == "all" ]]; then
  cutoff="$(date -u -d "-${RETENTION_DAYS} days" +%s)"
  for pat in \
    "week6-acceptance-*.log" \
    "week6-acceptance-*.json" \
    "week6-diagnostic-*.json" \
    "d13-benchmark-raw-*.json" \
    "d13-relation-status-*.json" \
    "d13-relation-collect-*.log" \
    "fault-injection-daily-*.json" \
    "rollback-daily-drill-*.json"; do
    while IFS= read -r -d '' f; do
      mtime="$(date -u -r "${f}" +%s)"
      if (( mtime < cutoff )); then
        iso="$(date -u -r "${f}" +%Y-%m-%dT%H:%M:%SZ)"
        safe_rm "${f}"
        extra="$(jq -cn --arg t "${iso}" '{last_write_time:$t}')"
        record_delete "runtime_old_detail" "${f}" "${extra}"
      fi
    done < <(find "${RUNTIME_DIR}" -maxdepth 1 -type f -name "${pat}" -print0 2>/dev/null || true)
  done

  if ! [[ "${BENCHMARK_KEEP_LATEST}" =~ ^[0-9]+$ ]]; then
    BENCHMARK_KEEP_LATEST=8
  fi
  if (( BENCHMARK_KEEP_LATEST < 1 )); then
    BENCHMARK_KEEP_LATEST=1
  fi
  prune_latest_by_pattern "d13-realbiz-benchmark-*.json" "${BENCHMARK_KEEP_LATEST}" "runtime_benchmark_overflow"
  prune_latest_by_pattern "d13-benchmark-raw-*.json" "${BENCHMARK_KEEP_LATEST}" "runtime_benchmark_overflow"
  prune_latest_by_pattern "benchmark_v1_eval_*.json" "${BENCHMARK_KEEP_LATEST}" "runtime_benchmark_overflow"
  prune_latest_by_pattern "benchmark_v1_compare_*.json" "${BENCHMARK_KEEP_LATEST}" "runtime_benchmark_overflow"
fi

report_json="$(jq -n \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg repo_root "${REPO_ROOT}" \
  --arg runtime_dir "${RUNTIME_DIR}" \
  --arg action "${ACTION}" \
  --argjson retention_days "${RETENTION_DAYS}" \
  --argjson apply "$([[ "${APPLY}" == "1" ]] && echo true || echo false)" \
  --argjson deleted "${deleted_json}" \
  --argjson kept "${kept_json}" \
  --argjson summary_files "${summary_json}" \
  --argjson errors "${errors_json}" \
  '{
    generated_at:$generated_at,
    repo_root:$repo_root,
    runtime_dir:$runtime_dir,
    action:$action,
    retention_days:$retention_days,
    apply:$apply,
    deleted:$deleted,
    kept:$kept,
    summary_files:$summary_files,
    errors:$errors
  }')"

if [[ "${PERSIST_REPORT}" == "1" ]]; then
  mkdir -p "${RETENTION_DIR}"
  printf '%s\n' "${report_json}" > "${REPORT_PATH}"
  echo "RUNTIME_RETENTION_REPORT=${REPORT_PATH}"
else
  echo "RUNTIME_RETENTION_REPORT=inline"
  printf '%s\n' "${report_json}"
fi
