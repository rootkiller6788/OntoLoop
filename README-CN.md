# OntoLoop 主权操作系统（工程运行时）
OntoLoop 是面向**治理 + 执行 + 学习 + 回放**的 AI 主权运行时。
当前版本已进入工程可用量产阶段，具备治理、执行、可信校验与持续学习全闭环能力。

## 核心能力模块
- 治理流水线：策略 / 租户 / 审批 / 风险 / 预算 / 灰度发布
- 执行流水线：能力准入 → 可信准入内核 → 运行时防护 → 分层工具栈 → 执行架构
- 可信流水线：证据标记器 → 流转状态引擎 → 可信证据账本（哈希链结构）
- 学习流水线：学习提案 → 晋升准入闸 → 私有记忆 / 组织知识库
- 可观测性流水线：遥测采集器 → 策略信号聚合器 → 查询/回放/解释平面
- 可信内核（本体核心）集成：身份认证、可信背书、供应链校验、强隔离、可回放审计

## 架构流水线（精简版）
```text
[用户/触发源]
    -> 结构化传输 + 会话桥接层
    -> 会话轮次状态机 + 上下文编译/压缩器
    -> 需求澄清模块
    -> 策略与治理上下文
    -> 知识上下文 + 超级记忆模块
    -> 查询引擎 + 编排调度器 + 能力路由器
    -> 能力准入校验
    -> 可信准入内核
    -> 运行时防护 + 权限模式管控
    -> 分层工具执行 + 执行架构 + 运行时钩子
    -> 证据标记采集
    -> 流转状态引擎
    -> 可信证据账本落盘
    -> 校验器与审计模块
    -> 模型学习与能力晋升
    -> 私有记忆 / 组织知识库更新
    -> 统一查询 / 流程回放 / 行为解释
    -> 可观测性监控 + 报表输出
    -> 进入下一轮迭代闭环
```

## 快速开始
### 1）环境依赖
- Rust 工具链
- 可选：状态存储命令行工具
- 可选：Docker / Docker Compose

### 2）本地运行
```powershell
cargo run --manifest-path .\Cargo.toml -- --message "构建可治理的自主运行闭环" --swarm
```

### 3）本地校验
```powershell
cargo check --workspace --manifest-path .\Cargo.toml
cargo test --workspace --manifest-path .\Cargo.toml
```

## 常用 CLI 命令
```powershell
# 系统健康自检
cargo run --manifest-path .\Cargo.toml -- system health

# 指定会话生成流程回放报告
cargo run --manifest-path .\Cargo.toml -- --session demo system replay-report

# 查看所有触发器列表
cargo run --manifest-path .\Cargo.toml -- trigger list

# 指定会话查看管控看板
cargo run --manifest-path .\Cargo.toml -- --session demo focus board

# 查看组织级上下文配置
cargo run --manifest-path .\Cargo.toml -- --session demo org context

# 查看桥接服务状态
cargo run --manifest-path .\Cargo.toml -- bridge status

# 批量导出知识图谱
cargo run --manifest-path .\Cargo.toml -- knowledge batch-export --anchor-list .\deploy\anchors.txt --type graph

# 触发 Webhook 事件并立即执行
cargo run --manifest-path .\Cargo.toml -- trigger webhook --anchor-id cli:focus --topic order.created --payload "{\"order_id\": \"A-1001\"}" --run-now

# 导出指定会话系统配置
cargo run --manifest-path .\Cargo.toml -- --session cli:focus system export

# 查看前端服务状态
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend status

# 查看前端事件日志（格式化输出、限制条数）
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend events --format pretty --limit 20
```

CLI 前端采用增量式扩展设计，现有 `dashboard-ui/` 等前端目录完整保留，可用于后续应用界面拓展开发。

## 验收测试脚本
Windows 环境：
```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\p95_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\pq9_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\week6_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\trigger_supermemory_acceptance.ps1
```

Linux 环境：
```bash
bash ./deploy/scripts/week6_acceptance.sh
```

输出产物：
- `deploy/runtime/p95-acceptance.log`
- `deploy/runtime/p95-acceptance.json`

## 项目目录结构
- `src/`：核心运行时与治理逻辑
- `src/query_engine/`：会话轮次 / 任务续接 / 上下文压缩 / 闭环调度器
- `src/runtime/`：运行时防护 / 准入控制 / 任务执行 / 证据采集 / 流转引擎
- `src/security/`：策略管控 / 权限模式 / 能力准入体系
- `src/session/`：检查点 / 任务恢复 / 运行时会话管理
- `src/memory/`：私有记忆 + 超级记忆流水线
- `src/observability/`：遥测采集器 / 查询平面 / 流程回放与链路解释
- `src/transport/`：结构化传输 / 跨环境会话桥接
- `src/plugins/`：插件生命周期管理
- `src/skills/`：技能注册中心 / 能力构建流水线
- `src/services/`：服务中介与编排核心骨架
- `tests/`：端到端测试 & 回归测试套件
- `deploy/scripts/`：自动化验收与运维脚本
- `docs/`：协议规范文档 / 架构设计文档 / 验收标准文档

## 当前版本状态
当前发布版已达到高成熟工程阶段：**核心流水线可完整运行、数据全链路可追溯、审计日志可解释**。
建议优先通过 `deploy/scripts/` 完成全量回归校验，再逐步落地生产级业务负载。

命令行规范文档路径：
- docs/CLI_SPECIFICATION.md

## 白盒信号调试命令（D9/D10/D11 层级）
内部信号流水线对外开放白盒 CLI 调试命令，用于问题排查与链路观测：
```powershell
# 查看系统信号链路状态
cargo run --manifest-path .\Cargo.toml -- system signal status

# 根据追踪 ID 解析信号链路详情
cargo run --manifest-path .\Cargo.toml -- system signal explain --trace-id <trace-id>

# 清空信号缓存队列
cargo run --manifest-path .\Cargo.toml -- system signal drain
```

### 开发治理规范
- 所有业务侧信号写入，必须统一经过 `SignalFacade` 门面层。
- 绕过门面层的直写操作，将被静态扫描测试拦截禁止。
- 信号流水线验收已集成至 `week6_acceptance` 全量回归脚本。

## 规则引用策略
`rule/` 目录下的第三方参考资料，仅用于架构对标与理论抽象提炼。
该目录文件不参与项目编译依赖，也不会在运行时被加载执行。