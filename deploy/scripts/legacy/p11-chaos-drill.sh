#!/usr/bin/env bash
set -euo pipefail

CONFIG="${1:-deploy/config/autoloop.dev.toml}"
SESSION="${2:-p11-chaos-drill}"

echo "[P11] Inject provider-down chaos"
cargo run -- --config "$CONFIG" --session "$SESSION" system chaos --fault provider_down --profile provider_fallback

echo "[P11] Inject mcp-timeout chaos"
cargo run -- --config "$CONFIG" --session "$SESSION" system chaos --fault mcp_timeout --profile mcp_conservative

echo "[P11] Inject queue congestion chaos"
cargo run -- --config "$CONFIG" --session "$SESSION" system chaos --fault queue_congestion --profile queue_throttle

echo "[P11] Recover session"
cargo run -- --config "$CONFIG" --session "$SESSION" system recover --reason "chaos drill completed"

echo "[P11] Resilience report"
cargo run -- --config "$CONFIG" --session "$SESSION" knowledge export --type resilience
