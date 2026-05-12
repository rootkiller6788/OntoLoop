param(
  [string]$Config = "deploy/config/autoloop.dev.toml",
  [string]$Session = "acceptance-trigger-supermemory"
)

$ErrorActionPreference = "Stop"

Write-Host "[1/6] cargo check"
cargo check --manifest-path D:\AutoLoop\autoloop-app\Cargo.toml

Write-Host "[2/6] trigger route kind unit test"
cargo test --manifest-path D:\AutoLoop\autoloop-app\Cargo.toml --lib runtime::trigger_runtime::tests::classify_trigger_kind_detects_runtime_modes

Write-Host "[3/6] webhook ingress unit test"
cargo test --manifest-path D:\AutoLoop\autoloop-app\Cargo.toml --lib runtime::trigger_runtime::tests::webhook_ingress_normalizes_topic_and_executes_in_worker

Write-Host "[4/6] supermemory stage5 e2e"
cargo test --manifest-path D:\AutoLoop\autoloop-app\Cargo.toml --test p14_supermemory_stage5_e2e

Write-Host "[5/6] day10 acceptance e2e"
cargo test --manifest-path D:\AutoLoop\autoloop-app\Cargo.toml --test p10_day10_acceptance_e2e

Write-Host "[6/6] command-level daemon linkage check"
$setArgs = @(
  "run",
  "--manifest-path", "D:\AutoLoop\autoloop-app\Cargo.toml",
  "--",
  "--config", $Config,
  "trigger", "set",
  "--anchor-id", $Session,
  "--schedule", "trigger:once:acceptance",
  "--payload", "acceptance_payload"
)
$setProc = Start-Process -FilePath "cargo" -ArgumentList $setArgs -NoNewWindow -PassThru
$setProc.WaitForExit()

$tmpOut = Join-Path $env:TEMP ("autoloop-trigger-run-out-" + [guid]::NewGuid().ToString() + ".log")
$tmpErr = Join-Path $env:TEMP ("autoloop-trigger-run-err-" + [guid]::NewGuid().ToString() + ".log")
$runArgs = @(
  "run",
  "--manifest-path", "D:\AutoLoop\autoloop-app\Cargo.toml",
  "--",
  "--config", $Config,
  "trigger", "run",
  "--anchor-id", $Session
)
$runProc = Start-Process -FilePath "cargo" -ArgumentList $runArgs -NoNewWindow -PassThru -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr
$runProc.WaitForExit()
$run = (Get-Content -Raw $tmpOut) + "`n" + (Get-Content -Raw $tmpErr)
Remove-Item -Force $tmpOut
Remove-Item -Force $tmpErr

if ($run -notmatch '"supermemory_queue"') {
  throw "trigger output missing supermemory_queue block"
}
if ($run -notmatch '"trigger_report"') {
  throw "trigger output missing trigger_report block"
}
if ($run -notmatch ('"session_id"\s*:\s*"' + [regex]::Escape($Session) + '"')) {
  throw "trigger output missing target session id"
}

Write-Host "Acceptance passed: trigger daemon default linkage with supermemory queue is active."
