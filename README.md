# OntoLoop Sovereign OS (Engineering Runtime)

OntoLoop 鏄竴涓潰鍚戔€滄不鐞?+ 鎵ц + 瀛︿範 + 鍥炴斁鈥濈殑 AI 涓绘潈杩愯鏃躲€傚綋鍓嶇増鏈凡缁忚繘鍏ュ彲楠屾敹宸ョ▼鎬侊紝鍏峰娌荤悊闂幆銆佹墽琛岄棴鐜€佽瘉鎹棴鐜拰瀛︿範闂幆銆?
## 鎴戜滑鐜板湪鏈夊摢浜涜兘鍔?
- 娌荤悊涓婚摼锛歅olicy / Tenant / Approval / Risk / Budget / Rollout
- 鎵ц涓婚摼锛欳apability Admission -> Trust Admission Kernel -> Runtime Guard -> Layered Tool Stack -> Execution Fabric
- 璇佹嵁涓婚摼锛欵vidence Tagger -> Flow State Engine -> Trust Evidence Ledger锛坔ash-chain锛?- 瀛︿範涓婚摼锛歀earning Proposal -> Promotion Gate -> Private Memory / Org Knowledge
- 鍙娴嬩富閾撅細Telemetry Collector -> Policy Signal Aggregator -> Query/Replay/Explanation Plane
- TrustKernel锛圢oumenonCore锛夎瀺鍚堬細韬唤璇佹槑銆乤ttestation銆佷緵搴旈摼楠岃瘉銆佸己鍒堕棬绂併€佸彲鍥炴斁瀹¤

## 鏋舵瀯涓婚摼锛堢畝鐗堬級

```text
[User/Trigger]
    -> [Structured Transport + Session Bridge]
    -> [Query Turn State Machine + Context Compiler/Compactor]
    -> [Requirement Clarification]
    -> [Policy & Governance Context]
    -> [Knowledge Context + SuperMemory]
    -> [QueryEngine + Orchestrator + Capability Router]
    -> [Capability Admission]
    -> [Trust Admission Kernel]
    -> [Runtime Guard + Permission Mode]
    -> [Layered Tool Execution + Execution Fabric + Hook Runtime]
    -> [Evidence Tagger]
    -> [Flow State Engine]
    -> [Trust Evidence Ledger]
    -> [Verifier & Audit]
    -> [Learning + Promotion]
    -> [Memory / Org Knowledge Update]
    -> [Unified Query / Replay / Explanation]
    -> [Observability + Reports]
    -> [Next Iteration]
```

## 蹇€熷紑濮?
### 1) 鐜瑕佹眰

- Rust toolchain
- 鍙€夛細StateStore CLI
- 鍙€夛細Docker / Docker Compose

### 2) 鏈湴杩愯

```powershell
cargo run --manifest-path .\Cargo.toml -- --message "Build a governed autonomous loop" --swarm
```

### 3) 鏈湴妫€鏌?
```powershell
cargo check --workspace --manifest-path .\Cargo.toml
cargo test --workspace --manifest-path .\Cargo.toml
```

## 甯哥敤鍛戒护

```powershell
cargo run --manifest-path .\Cargo.toml -- system health
cargo run --manifest-path .\Cargo.toml -- --session demo system replay-report
cargo run --manifest-path .\Cargo.toml -- trigger list
cargo run --manifest-path .\Cargo.toml -- --session demo focus board
cargo run --manifest-path .\Cargo.toml -- --session demo org context
cargo run --manifest-path .\Cargo.toml -- bridge status
cargo run --manifest-path .\Cargo.toml -- knowledge batch-export --anchor-list .\deploy\anchors.txt --type graph
cargo run --manifest-path .\Cargo.toml -- trigger webhook --anchor-id cli:focus --topic order.created --payload "{""order_id"": ""A-1001""}" --run-now
cargo run --manifest-path .\Cargo.toml -- --session cli:focus system export
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend status
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend events --format pretty --limit 20
```

CLI frontend is additive. Existing app frontend directories such as `dashboard-ui/` are preserved for later application surfaces.

## 楠屾敹鑴氭湰

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\p95_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\pq9_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\week6_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\trigger_supermemory_acceptance.ps1
```
Linux: `bash ./deploy/scripts/week6_acceptance.sh`

杈撳嚭璇佹嵁锛?
- `deploy/runtime/p95-acceptance.log`
- `deploy/runtime/p95-acceptance.json`

## 鐩綍缁撴瀯

- `src/`锛氭牳蹇冭繍琛屾椂涓庢不鐞嗛€昏緫
- `src/query_engine/`锛歵urn state / continuation / compactor / loop
- `src/runtime/`锛歡uard / admission / execution / evidence / flow
- `src/security/`锛歱olicy / permission mode / capability admission
- `src/session/`锛歝heckpoint / resume / runtime
- `src/memory/`锛歮emory + supermemory pipeline
- `src/observability/`锛歝ollector / query plane / replay explanation
- `src/transport/`锛歴tructured transport + bridge
- `src/plugins/`锛歱lugin lifecycle
- `src/skills/`锛歴kill registry / builder pipeline
- `src/services/`锛歴ervice mediation spine
- `tests/`锛欵2E + 鍥炲綊娴嬭瘯
- `deploy/scripts/`锛氶獙鏀朵笌婕旂粌鑴氭湰
- `docs/`锛氬崗璁€佹灦鏋勩€侀獙鏀舵枃妗?
## 褰撳墠鐘舵€?
褰撳墠鐗堟湰澶勪簬鈥滀富閾惧彲璺戙€佽瘉鎹彲杩芥函銆佸洖鏀惧彲瑙ｉ噴鈥濈殑楂樺彲鐢ㄥ伐绋嬮樁娈碉紱寤鸿鎸佺画鎸?`deploy/scripts/` 鍥炲綊鍚庡啀閫愭鏀惧ぇ鐢熶骇娴侀噺銆?


CLI 瑙勮寖鏂囨。锛?
- docs/CLI_SPECIFICATION.md

## Signal Whitebox Commands (D9/D10/D11)

The signal pipeline is now exposed through whitebox CLI commands:

```powershell
cargo run --manifest-path .\Cargo.toml -- system signal status
cargo run --manifest-path .\Cargo.toml -- system signal explain --trace-id <trace-id>
cargo run --manifest-path .\Cargo.toml -- system signal drain
```

Implementation policy:
- Business-side signal writes must go through `SignalFacade`.
- Direct bypass writes are blocked by static scan tests.
- Signal acceptance is included in `deploy/scripts/week6_acceptance.ps1` and `deploy/scripts/week6_acceptance.sh`.

## Rule Reference Policy

`rule/` 下的第三方材料如需引入，仅用于设计对照与第一性抽取，不进入构建与运行时依赖。



