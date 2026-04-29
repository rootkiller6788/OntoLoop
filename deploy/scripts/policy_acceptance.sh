#!/usr/bin/env bash
set -euo pipefail

MANIFEST_PATH="${1:-./Cargo.toml}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
RUNTIME_DIR="${REPO_ROOT}/deploy/runtime"
STAMP="$(date +%Y%m%d-%H%M%S)"
LOG_PATH="${RUNTIME_DIR}/policy-acceptance-${STAMP}.log"
JSON_PATH="${RUNTIME_DIR}/policy-acceptance-${STAMP}.json"

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
run_step "bundle-signature-mismatch-reject" cargo test --manifest-path "${MANIFEST_PATH}" --test policy_acceptance_e2e bundle_signature_mismatch_rejected
run_step "discovery-fetch-failure-auto-rollback" cargo test --manifest-path "${MANIFEST_PATH}" --test policy_acceptance_e2e discovery_fetch_failure_auto_rollback_keeps_stable_current
run_step "enforced-high-risk-deny" cargo test --manifest-path "${MANIFEST_PATH}" --test policy_acceptance_e2e enforced_mode_high_risk_deny_effective
run_step "shadow-diff-traceable" cargo test --manifest-path "${MANIFEST_PATH}" --test policy_acceptance_e2e shadow_mode_diff_traceable
run_step "mask-drop-no-sensitive-leak" cargo test --manifest-path "${MANIFEST_PATH}" --test policy_acceptance_e2e mask_drop_logs_do_not_leak_sensitive_fields

RESULTS_JSON="[${RESULTS_JSON%,}]"

cat > "${JSON_PATH}" <<EOF
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "repo_root": "${REPO_ROOT}",
  "manifest": "${MANIFEST_PATH}",
  "all_passed": true,
  "required_checks": [
    "bundle 签名错误拒绝",
    "discovery 拉取失败自动回滚",
    "enforced 模式高风险 deny 生效",
    "shadow 模式差异可追溯",
    "mask/drop 后日志不泄露敏感字段"
  ],
  "commands": ${RESULTS_JSON},
  "log_path": "${LOG_PATH}"
}
EOF

echo "POLICY_ACCEPTANCE_OK log=${LOG_PATH}"
echo "POLICY_ACCEPTANCE_JSON=${JSON_PATH}"
