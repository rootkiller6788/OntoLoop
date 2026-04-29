# 对话操作记录（从起始到当前）

## 0. 说明
- 本文档基于本次对话的需求推进顺序 + 当前仓库可见代码状态整理。
- 目标是帮助快速理解「我们做了什么、改了哪些文件、现在到哪一步」。

## 1. 目标收敛与边界冻结（早期阶段）
- 将 TrustKernel 收缩为「可信执行强制内核」。
- 吸收并固定了你提出的三条关键修正：
  - `ReplayCheck` 保留为能力，但 `replay` 不再作为内核运行模式。
  - `governance` 仅保留强制门禁（mandatory enforcement + rollback gate）。
  - 状态机加入过程态并收紧终态语义。
- 关键落盘：`ARCHITECTURE.md` 中已写入边界句与不变量约束（含 `ReplayCheck != mode`）。

## 2. 按里程碑执行（M1~M6 / P0~P5）

### P0 规范冻结
- 文件：`ARCHITECTURE.md`
- 已固化：
  - `Admitted iff ...` 准入条件
  - `execution_fingerprint` 公式
  - `I1-I8` 不变量
  - `ReplayCheck != execution mode`

### P1 供应链准入真实化（Sigstore/cosign）
- 文件：`src/admission_sigstore.rs`（新增）、`src/trust.rs`、`src/kernel.rs`
- 操作：
  - 新增 Sigstore 准入验证器（签名、Rekor、provenance、policy pin）。
  - 内核准入前置检查，失败即 `Rejected`，且不进入 evidence 写入路径。

### P2 闭包指纹强化（Nix 风格）
- 文件：`src/ir.rs`、`src/kernel.rs`、`src/replay.rs`
- 操作：
  - 将执行指纹升级为闭包级身份摘要，覆盖 flake/derivation/store paths/runtime closure/policy/plugin-verifier/config/generation 等材料。
  - `replay_check` 使用同一公式重算并比对。

### P3 不变量测试层
- 文件：`src/state.rs`、`src/lib.rs`、`tests/invariants.rs`（新增）
- 操作：
  - 收紧状态机迁移矩阵与门禁（side-effect gate、ledger gate、失败到回滚）。
  - 增加 I1-I8 对应自动化测试。

### P4 高风险边界硬化
- 文件：`src/ir.rs`、`src/kernel.rs`、`src/resource.rs`
- 操作：
  - 对 wasm/FFI/plugin 路径强制 `hardening_profile` 与资源/系统调用约束。
  - 缺失约束直接拒绝执行（Rejected/Fail->Rollback 路径受控）。

### P5 发布门禁与 CI
- 文件：`Cargo.toml`、`.github/workflows/release-gates.yml`（新增）
- 操作：
  - 建立 core/extensions + invariants 的门禁测试矩阵。
  - 将关键安全门禁测试加入发布前检查链路。

## 3. 命名改造：TrustKernel -> NoumenonCore
- 操作：
  - 代码与文档核心命名已改为 `NoumenonCore`（结构体、导出、架构文档、描述文本）。
- 现状检查：
  - 在 `ARCHITECTURE.md`、`src/*`、`tests/*`、`.github/workflows/*`、`Cargo.toml` 中检索 `TrustKernel`，未发现残留。
- 备注：
  - crate 包名当前仍是 `trustkernel`（`Cargo.toml` 的 `[package].name`），若要统一，可继续改为 `noumenoncore` 并同步下游引用。

## 4. 当前可核验状态（本轮再次验证）
- 本轮执行了 `cargo test`，结果：
  - 单元测试：`40 passed, 0 failed`
  - 不变量测试：`8 passed, 0 failed`
  - 总体：全部通过
- 关键测试名可见：
  - `rejects_non_mandatory_operation_modes`
  - `rejects_sigstore_failure_without_evidence`
  - `rejects_wasm_path_without_hardening_profile`
  - `rejects_wasm_path_without_syscall_constraints`
  - `completed_implies_validate_pass_contract`
  - `i1..i8` 不变量测试集合

## 5. 结果总结（一句话）
- 这次对话已把内核从“功能集合”推进为“强制边界机器”：准入必须可信、执行必须受控、证据必须可验证、回放必须可重算。
