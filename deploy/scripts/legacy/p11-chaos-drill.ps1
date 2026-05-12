$ErrorActionPreference = "Stop"

param(
    [string]$Config = "deploy/config/autoloop.dev.toml",
    [string]$Session = "p11-chaos-drill"
)

Write-Host "[P11] Inject provider-down chaos"
cargo run -- --config $Config --session $Session system chaos --fault provider_down --profile provider_fallback

Write-Host "[P11] Inject mcp-timeout chaos"
cargo run -- --config $Config --session $Session system chaos --fault mcp_timeout --profile mcp_conservative

Write-Host "[P11] Inject queue congestion chaos"
cargo run -- --config $Config --session $Session system chaos --fault queue_congestion --profile queue_throttle

Write-Host "[P11] Recover session"
cargo run -- --config $Config --session $Session system recover --reason "chaos drill completed"

Write-Host "[P11] Resilience report"
cargo run -- --config $Config --session $Session knowledge export --type resilience
