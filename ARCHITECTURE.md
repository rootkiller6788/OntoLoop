# AutoLoop Architecture

This document is a concise map of the current system using neutral engineering terms.

## Runtime Flow

1. CLI receives intent (`src/main.rs`).
2. `AutoLoopApp` assembles runtime subsystems (`src/lib.rs`).
3. Orchestration runs intent clarification -> strategy planning -> swarm execution (`src/orchestration/mod.rs`).
4. Runtime guard and verifier enforce bounded execution (`src/runtime/mod.rs`).
5. Knowledge and learning artifacts persist through the StateStore adapter.

## Core Modules

- `src/orchestration/`
  - Intent clarification, planning, debate rounds, route selection, validation.
- `src/runtime/`
  - Runtime guard, circuit breaker state, evaluation, verifier logic.
- `src/providers/`
  - OpenAI-compatible HTTP provider abstraction and model routing.
- `src/tools/`
  - Tool registry and forged capability catalog.
- `src/research/`
  - Anchor-driven research execution backends and data acquisition.
- `src/rag/`
  - GraphRAG snapshot/update/retrieval and graph signals.
- `src/memory/`
  - Learning assets and memory retrieval/consolidation.
- `src/observability/`
  - Route analytics, failure forensics, dashboard artifacts.
- `src/dashboard_server.rs`
  - Minimal HTTP + SSE backend for snapshot/replay/governance UX.

## Data and Storage

- Primary runtime record layer: `autoloop-state-adapter/`
- StateStore module crate: `state_store/`
- Local runtime artifacts: `deploy/runtime/`

## Frontend Control Surface

- Location: `dashboard-ui/`
- Stack: Vue 3 + TypeScript + Vite
- Features:
  - Capability operations
  - Session replay
  - Graph overlays
  - SSE event updates
  - Operator settings (language/vendor/base URL/model/API key)

## Deployment Surfaces

- Local scripts and templates: `deploy/`
- K8s manifests: `deploy/k8s/`
- Monitoring templates: `deploy/monitoring/`
- One-command startup scripts:
  - `deploy/scripts/start-autoloop.ps1`
  - `deploy/scripts/start-autoloop.sh`

## Signal Plane (Whitebox)

- Internal write path is `SignalFacade` -> signal pipeline (no bypass).
- Pipeline behavior includes redact/filter/sample/rate-limit, dual sink (`evidence` primary + `query-plane explain` secondary), batch/retry/backoff/shutdown flush.
- CLI whitebox surface:
  - `system signal status`
  - `system signal explain`
  - `system signal drain`

## Reference-Only Components

`rule/` 目录仅用于参考资料管理，不参与编译链接或运行时启动路径。

