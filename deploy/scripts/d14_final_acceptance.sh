#!/usr/bin/env bash
set -euo pipefail

RUNTIME_DIR="${1:-deploy/runtime}"
WEEK6_JSON="${2:-}"
DAILY_RELEASE_PACKAGE_JSON="${3:-}"
RELEASE_GATE_JSON="${4:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ "${RUNTIME_DIR}" = /* || "${RUNTIME_DIR}" =~ ^[A-Za-z]:\\ ]]; then
  RESOLVED_RUNTIME="${RUNTIME_DIR}"
else
  RESOLVED_RUNTIME="${REPO_ROOT}/${RUNTIME_DIR}"
fi
mkdir -p "${RESOLVED_RUNTIME}"

resolve_input() {
  local candidate="$1"
  local fallback="$2"
  if [[ -n "${candidate}" ]]; then
    echo "${candidate}"
  else
    echo "${RESOLVED_RUNTIME}/${fallback}"
  fi
}

WEEK6_PATH="$(resolve_input "${WEEK6_JSON}" "week6_full_acceptance.json")"
DAILY_PATH="$(resolve_input "${DAILY_RELEASE_PACKAGE_JSON}" "daily_release_package.json")"
GATE_PATH="$(resolve_input "${RELEASE_GATE_JSON}" "release_gate.json")"

for required in "${WEEK6_PATH}" "${DAILY_PATH}" "${GATE_PATH}"; do
  if [[ ! -f "${required}" ]]; then
    echo "d14 final acceptance missing required input: ${required}" >&2
    exit 1
  fi
done

WEEK6_WIN="$(cygpath -w "${WEEK6_PATH}")"
DAILY_WIN="$(cygpath -w "${DAILY_PATH}")"
GATE_WIN="$(cygpath -w "${GATE_PATH}")"
RUNTIME_WIN="$(cygpath -w "${RESOLVED_RUNTIME}")"

OUT="$(
  powershell -NoProfile -Command "\
  \$week6Path = '${WEEK6_WIN}'; \
  \$dailyPath = '${DAILY_WIN}'; \
  \$gatePath = '${GATE_WIN}'; \
  \$runtimeWin = '${RUNTIME_WIN}'; \
  \$week6 = Get-Content -Raw -Path \$week6Path | ConvertFrom-Json; \
  \$daily = Get-Content -Raw -Path \$dailyPath | ConvertFrom-Json; \
  \$gate = Get-Content -Raw -Path \$gatePath | ConvertFrom-Json; \
  \$versionAPath = [string]\$week6.version_a_report; \
  \$d14Path = [string]\$week6.d14_rollout_report; \
  if ([string]::IsNullOrWhiteSpace(\$versionAPath) -or -not (Test-Path \$versionAPath)) { throw 'd14 final acceptance missing version_a_report path from week6' }; \
  if ([string]::IsNullOrWhiteSpace(\$d14Path) -or -not (Test-Path \$d14Path)) { throw 'd14 final acceptance missing d14_rollout_report path from week6' }; \
  \$versionA = Get-Content -Raw -Path \$versionAPath | ConvertFrom-Json; \
  \$d14 = Get-Content -Raw -Path \$d14Path | ConvertFrom-Json; \
  \$checks = @(\$week6.checks); \
  \$commands = @(\$d14.commands); \
  function Get-CheckPassed([string]\$id) { \$item = \$checks | Where-Object { \$_.check_id -eq \$id } | Select-Object -First 1; if (\$null -eq \$item) { return \$false }; return (\$item.passed -eq \$true) }; \
  function Has-CommandPassed([string]\$name) { \$cmd = \$commands | Where-Object { \$_.name -eq \$name } | Select-Object -First 1; if (\$null -eq \$cmd) { return \$false }; return (\$cmd.passed -eq \$true) }; \
  \$contract = (\$versionA.all_passed -eq \$true); \
  \$gateOk = (\$daily.allow_release -eq \$true) -and (\$gate.allow_release -eq \$true); \
  \$lease = Get-CheckPassed 'no-bypass-kernel-mediator-static'; \
  \$review = Get-CheckPassed 'artifact-hard-gate-and-proof'; \
  \$wal = (Get-CheckPassed 'waltx-production-write-minimal-e2e') -and (Get-CheckPassed 'd12-storage-postgres-wal-dualwrite-replay'); \
  \$ontoevent = (Get-CheckPassed 'signal-acceptance') -and (Get-CheckPassed 'frontend-cli-acceptance'); \
  \$releasePkg = Test-Path \$dailyPath; \
  \$shadow = (Has-CommandPassed 'rollout-shadow-status') -and (Has-CommandPassed 'rollout-shadow-health'); \
  \$canary10 = (Has-CommandPassed 'rollout-canary10-status') -and (Has-CommandPassed 'rollout-canary10-health'); \
  \$canary30 = (Has-CommandPassed 'rollout-canary30-status') -and (Has-CommandPassed 'rollout-canary30-health'); \
  \$full = (Has-CommandPassed 'rollout-full-status') -and (Has-CommandPassed 'rollout-full-health'); \
  \$rollback = (\$d14.rollback.triggered -eq \$true) -and (\$d14.rollback.status_ok -eq \$true) -and (\$d14.rollback.health_ok -eq \$true); \
  \$rolloutAll = \$shadow -and \$canary10 -and \$canary30 -and \$full -and \$rollback; \
  \$deny = New-Object System.Collections.Generic.List[string]; \
  if (-not \$contract) { \$deny.Add('contract_failed') }; \
  if (-not \$gateOk) { \$deny.Add('gate_failed') }; \
  if (-not \$lease) { \$deny.Add('lease_guard_missing_or_failed') }; \
  if (-not \$review) { \$deny.Add('reviewgate_failed') }; \
  if (-not \$wal) { \$deny.Add('wal_failed') }; \
  if (-not \$ontoevent) { \$deny.Add('ontoevent_chain_incomplete') }; \
  if (-not \$releasePkg) { \$deny.Add('release_package_missing') }; \
  if (-not \$rolloutAll) { \$deny.Add('rollout_chain_incomplete') }; \
  \$all = (\$deny.Count -eq 0); \
  \$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'; \
  \$outPath = Join-Path \$runtimeWin ('d14-final-acceptance-' + \$stamp + '.json'); \
  \$canonicalPath = Join-Path \$runtimeWin 'd14_final_acceptance.json'; \
  \$summary = [pscustomobject]@{ \
    generated_at = (Get-Date).ToString('s'); \
    version = 'd14-final/v1'; \
    all_passed = \$all; \
    deny_reasons = \$deny; \
    inputs = [pscustomobject]@{ \
      week6 = \$week6Path; \
      daily_release_package = \$dailyPath; \
      release_gate = \$gatePath; \
      version_a = \$versionAPath; \
      d14_rollout = \$d14Path; \
    }; \
    chain = [pscustomobject]@{ \
      contract = \$contract; \
      gate = \$gateOk; \
      lease = \$lease; \
      reviewgate = \$review; \
      wal = \$wal; \
      ontoevent = \$ontoevent; \
      release_package = \$releasePkg; \
      rollout = [pscustomobject]@{ \
        shadow = \$shadow; \
        canary10 = \$canary10; \
        canary30 = \$canary30; \
        full = \$full; \
        rollback = \$rollback; \
        all = \$rolloutAll; \
      }; \
    }; \
  }; \
  (\$summary | ConvertTo-Json -Depth 12) | Set-Content -Path \$outPath -Encoding utf8; \
  (\$summary | ConvertTo-Json -Depth 12) | Set-Content -Path \$canonicalPath -Encoding utf8; \
  Write-Output ('D14_FINAL_ACCEPTANCE_JSON=' + \$outPath); \
  Write-Output ('D14_FINAL_ACCEPTANCE_CANONICAL_JSON=' + \$canonicalPath); \
  if (-not \$all) { exit 1 }\
  "
)"

printf '%s\n' "${OUT}"
