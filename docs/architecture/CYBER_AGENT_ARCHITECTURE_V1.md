# Cyber Agent Architecture v1.0.0 — Frozen Baseline

## 0. 封存元数据

- 架构版本：`1.0.0`
- 封存日期：`2026-08-08`
- 状态：`FROZEN`
- 源设计：仓库根目录 `task.md`
- 源设计 SHA-256：`31b1cca7205ccce86cc30be263648966c008515b3cb26aee8645cb06bb28a2a5`
- Codex 参考提交：`3aae5d885bac39c1262491aa3fd100dfd8b3919f`
- 参考分析：`Codex架构与漏洞挖掘Agent复用评估.md`

本文件封存 Cyber Agent v1 的架构契约。它不是 `task.md` 的逐字复制，而是可独立实施和验收的规范化基线。

封存后不直接修改本文件。需要改变规范时，新建 ADR，并发布 `CYBER_AGENT_ARCHITECTURE_V1_1.md` 或下一主版本。

## 1. 产品边界

Cyber Agent 是单人使用、单机运行、本地优先的漏洞挖掘专用 Agent。

必须保留：

- 单主 Agent 和单一采样/工具循环。
- 单 Rust package、单默认二进制。
- 项目隔离、长期记忆、Research Ledger 和本地审计。
- 默认 FullAccess，不强制 Sandbox 或工具确认。
- Shell、文件、网络、设备、PTY 和长运行 Job。
- 文本与图片输入。
- Context Budget、Compaction、Snapshot/Diff 和增量 Provider 请求。
- CLI 优先，本地 Web 仅作可选 feature。
- 不上传 telemetry、analytics、feedback、crash dump 或产品日志。

明确不做：

- 多 Agent、团队、租户、权限和审批平台。
- 云 Server、远程任务、云同步和账号系统。
- Marketplace、通用插件框架和动态 Rust ABI。
- Guardian、Policy Engine 和跨平台 Sandbox 全集。
- 通用 Workflow DSL、知识图谱数据库、默认向量数据库和分布式队列。
- 完整 Codex TUI、App Server、Connectors、Cloud 或 Review 产品线。

## 2. 总体结构

    CLI / optional local Web
              │
              ▼
        Application Runtime
          ├── AgentLoop
          ├── ProviderSession
          ├── ContextPlanner
          ├── ProjectManager
          ├── MemoryManager
          ├── ResearchService
          ├── ToolRuntime
          ├── AuditWriter
          └── Replay/Eval Service (P1)
              │
              ▼
     Ports: Model / Tool / Event / Artifact / Cache
              │
              ▼
     Adapters: OpenAI / FS / Process / PTY / Image / JSONL

内部依赖方向：

    ui/main -> app -> ports -> domain
       │                ↑        ↑
       └-------> adapters -------┘

    domain / app / adapters -> support

约束：

- `domain` 只放类型、纯 Reducer、去重、Fingerprint 和协议语义。
- `ports` 只依赖 `domain`。
- `app` 只通过 Port 访问 I/O。
- `adapters` 不依赖 `app`。
- `ui/main` 是 Composition Root。
- 不创建 Cargo workspace，不为模块边界拆 crate。

## 3. 单 Agent 循环

    receive user input
      -> load TurnContext and ProviderSession
      -> append local UserMessage event
      -> refresh cheap Project/Runtime fingerprints
      -> capture StepSnapshot and active TargetSnapshot
      -> reduce Research state and calculate diffs
      -> build ContextPlan + ContextDecisionManifest
      -> build provider-neutral ModelInputIR
      -> choose full or append-only delivery
      -> stream ModelEvents
      -> resolve ToolExecutionPlan when tool is requested
      -> persist execution intent
      -> schedule using Resource Keys
      -> execute with FullAccess
      -> persist ExecutionReceipt and ToolResult
      -> create Artifact/Observation references
      -> update Research/Project/Job state
      -> append ToolResult and changed fragments
      -> sample again or finish turn

固定处理点只有：

- `before_model_request`
- `after_tool_result`
- `turn_end`

它们是直接函数或轻量中间件，不是动态 Hook Runtime。

## 4. 数据事实源

| 数据 | 权威来源 | 可删除派生视图 |
|---|---|---|
| 项目配置 | `project.toml`、`tools.toml`、Mode Pack | 解析缓存 |
| 对话、请求和工具审计 | Session `events.jsonl` | History、Summary、Metrics |
| Target/Hypothesis/Experiment/Finding/Action | `research.jsonl` | ResearchState、TargetDelta |
| 长期记忆 | `memory/*.md` | 检索索引、注入片段 |
| Artifact 身份与派生关系 | Artifact Event/Record | Lineage Graph |
| 原始字节 | content-addressed SHA-256 Blob Store | 缩略图、摘要 |
| Project/Tool Snapshot | 项目文件和工具探测 | `cache/` |
| Context Fragment | Prompt、Memory、Research、History | Fragment Cache |

规则：

- Cache、Summary、Snapshot、TargetDelta 和 Lineage Graph 都不能反向覆盖事实源。
- Session Event 可以引用 Research Event 和 Artifact ID，不复制另一事实源的完整 Payload。
- Blob ID 表示内容；Artifact ID 表示一次有语义和来源的使用。
- 同字节不同来源不能合并 Artifact Provenance。

## 5. Event、Reducer 与演进

    EventEnvelope<T> {
        schema_family
        schema_version
        event_id
        stream_id
        sequence
        occurred_at
        event_type
        payload
    }

核心规则：

- 单 Stream sequence 严格递增。
- 每个项目只有一个可写进程；使用 OS advisory lock。
- 每个 Stream 一个 Writer，工具并发不能直接并发写 JSONL。
- Event 先持久化再进入纯 Reducer。
- JSONL 只自动恢复损坏尾部；非尾部损坏进入只读诊断。
- 原始 Event Stream 默认不原地迁移。

演进流水线：

    raw event
      -> envelope validation
      -> chained vN-to-vN+1 upcasters
      -> current event
      -> pure reducer

所有已发布 Schema 必须保留连续 Upcaster。缺失 Upcaster 或影响 State 的未知事件不能静默忽略。

## 6. Project、Memory 与 Research

每个目标项目拥有独立 `.cyber-agent/`。项目隔离的是上下文、记忆、Prompt、Session、Research 和 Artifact；它不是 Sandbox。

Memory：

- Working Memory：当前上下文。
- Session Memory：可重建 `summary.md`。
- Project Memory：`core.md`、`notes/`、`decisions.md`、`failed-paths.md`。
- Global Personal Memory 只存通用偏好，不自动吸收项目漏洞内容。
- 外部编辑 Memory 后，下次加载记录 Revision Blob 和 Diff。

Research Ledger 使用稳定 ID 和 append-only 事件：

- Hypothesis
- Experiment
- Finding
- Next Action
- TargetSnapshot
- ExperimentCapsule

确定性 `dedup_key` 和 `action_fingerprint` 只给出重复提示。模型相似度不能自动合并 Record。

## 7. 可复现研究内核

### 7.1 TargetSnapshot

TargetSnapshot 固定目标身份、Kind、Locator、Version、Build、目标 Artifact、设备和关键运行环境。

- Snapshot Fingerprint 不包含时间戳、分析工具版本和备注。
- 相同 Fingerprint 复用已有 Snapshot。
- 任一语义字段变化创建新 Snapshot，不静默覆盖。
- 无法确认的字段显式为 unknown。

### 7.2 ExperimentCapsule

Capsule 固定：

- TargetSnapshot ID
- normalized action
- execution IDs
- input/output Artifact IDs
- environment delta
- account prerequisites
- toolchain fingerprint
- ExecutionReceipt IDs

`reproduced/confirmed` Finding 必须引用实际成立的 TargetSnapshot 和证据。

### 7.3 Artifact Lineage

ArtifactRecord 至少保存：

- artifact_id 和 blob_id
- media type 和 size
- producer event/tool call
- target snapshot
- parent artifacts
- transform
- toolchain fingerprint

Lineage Graph 从 Event/Record 派生，不引入图数据库。MVP 不自动执行破坏性 Blob GC。

### 7.4 Observation 与 TargetDelta

领域工具可以返回 ObservationEnvelope，包括 Kind、位置、主体、入口、Source、Sink、前提、置信度、证据和 Extractor 版本。

- Observation 不是 Finding。
- 原始输出仍存 Event/Artifact。
- Observation Index 是可重建缓存。
- 只有显式 research update 才能把 Observation 引用提升为 Research Record。
- TargetDelta 是两个 TargetSnapshot/Observation 集合的派生差分。

## 8. Context Compiler

ContextFragment 包含 ID、Source、Priority、Content Hash、Token Estimate、Cache Policy 和 Content Items。

ContextPlan 包含：

- Base Instructions
- Input Items
- Tool Specs
- Output Contract
- Prompt Cache Key
- Context Fingerprint
- Full Logical Manifest
- Append-only Candidate
- ContextDecisionManifest

固定 Full Request 顺序：

1. Base Prompt
2. Personal Preferences
3. Project Prompt
4. Mode Prompt
5. ResearchWorldState
6. Project Snapshot
7. Project Memory Core
8. Retrieved Memory
9. Session Summary
10. Recent History
11. User Input
12. Turn-local Tool/Job updates

增量请求不得重排旧 Items。Summary 在活跃 Baseline 期间只在本地更新，不自动替换 Provider 上下文。

ContextDecisionManifest 为每个候选片段记录 include/truncate/artifact/exclude、Reason Code、Token 和 Baseline Relation。Planner/Estimator 版本进入 Fingerprint。

## 9. Model Provider

Domain 只使用 ModelInputIR：

- instructions
- content items
- tool specs
- output contract
- reasoning options
- semantic request properties
- full/append-only delivery
- logical input manifest
- context decision manifest

OpenAIResponsesAdapter 负责具体 Responses HTTP 编解码。Provider-specific 类型和 `previous_response_id` 不进入 Domain/Research/Context。

IncrementalBaseline 至少比较：

- ModelInputIR Schema
- Provider Capability Mapping
- Model Profile
- Instructions
- Tool Schema/Exposure
- Output Schema
- semantic request properties
- canonical Input Items

Resume、进程重启、Compaction、Profile/Prompt/Tool/IR/Capability 变化必须重置 Baseline。Provider 网络重试不能重复执行本地工具。

## 10. Tool Runtime

ToolSpec 必须声明：

- version、input/output schema
- direct/deferred/hidden exposure
- parallel/resource_locked/serial
- resource key template
- effect class
- retry policy
- determinism
- cache policy
- timeout 和 output policy

Effect 分类：read_only、local_mutation、external_mutation、interactive。

Retry：replay_safe、dedupe_by_key、never_auto_retry。

只有 read_only + deterministic + content_addressed 工具可以复用结果。Cache Key 必须包含 Tool Version、TargetSnapshot、Input、Environment 和语义参数。

`run_command` 无法判断副作用时使用 interactive/never_auto_retry。这不会引入确认或审批，只影响自动重试和缓存。

## 11. Snapshot、Hash 与缓存

- Project Fingerprint 首先使用 Git/mtime/size/path 等便宜元数据。
- 只对变化、读取或被 Research 引用的文件按需计算 BLAKE3。
- 长期 Artifact Blob 使用 SHA-256。
- Watcher 只能提示刷新，不是事实源。
- 每个 Cache 有 Schema、Source Fingerprint、失效原因、Hit/Miss 和删除重建路径。

## 12. 多模态

MVP 保留本地图片：验证格式和大小、保存原始 Artifact、按预算生成派生图片、将选中副本交给 Provider。

原图和派生图必须通过 Artifact Lineage 关联。Provider 不支持图片时明确失败，不能静默忽略。

不实现图片生成和语音交互。

## 13. 本地审计

Session Event 至少记录：

- User/Assistant/Model Request
- ToolExecutionPlanned/Started/Completed/Failed/Cancelled
- ExecutionReceipt
- ArtifactCreated/Derived
- Memory/Prompt changes
- Compaction/Recovery

RequestAuditManifest 记录 ModelInputIR、Provider Adapter/Capability、Instructions、Input、Tool/Output Schema 的有序 Hash 和内容引用。

首次出现的请求内容写入去重 Blob；完整原始 HTTP Snapshot 默认关闭。由 Event、Manifest 和 Blob 可以重建语义请求，但不保存 Authorization Header 或 API Key。

所有内容只写本地。云模型会收到被选入请求的 Prompt、图片和工具结果，但不存在额外 telemetry 或远程审计。

## 14. Replay 与 Eval

不建立第二套 Trace 事实源。TraceFixtureManifest 只引用原 Event、Artifact 和 Receipt。

模式：

- Mechanical Replay：Replay Provider + Replay Tool，零真实外部动作。
- Context Recompile：重算 ContextDecision，不把旧模型输出当作新 Prompt 质量结论。
- Shadow Model Eval：用户显式触发真实模型，Tool 固定 Replay；未匹配 Tool Call 立即停止。

Replay/Shadow 使用隔离 Eval Session，默认不写 Project Memory/Research，也不能作为真实漏洞证据。

## 15. Codex 复用边界

允许移植小型算法和测试：

- Tool Call/Result 配对与 History 归一化
- Context Fragment
- 输出截断
- 图片验证/缩放子集
- Streaming Responses 必需部分
- previous_response_id 增量判断
- Prompt Cache 稳定 Hash
- Snapshot/Diff 思路

禁止直接依赖：

- codex-core
- codex-tools
- codex-apply-patch
- codex-utils-pty
- app-server
- connectors/code-mode/cloud 产品层

PTY 直接基于 portable-pty 最小实现。Apply Patch 使用自有 Adapter。任何 PORT 必须记录上游提交、路径、测试、License、本地改动、依赖和同步成本。

## 16. 超前设计分层

P0/MVP：

- ModelInputIR
- TargetSnapshot/ExperimentCapsule
- Tool Effect/Receipt
- ArtifactRecord/Lineage
- Event Upcaster

P1/M3-M4：

- Context Decision/Recompile
- Trace Replay/Shadow
- ObservationEnvelope
- TargetDelta

P2/Deferred：

- Knowledge Graph DB
- Vector DB default
- Distributed execution
- Model routing server
- Dynamic plugin ABI
- Workflow DSL

## 17. 架构验收不变量

- 两个项目状态不串扰。
- Cache/Summary 删除后可从事实源恢复。
- Event 重放确定，旧 Schema 可连续 Upcast。
- 同一 Tool Call 不因 Provider retry 重复执行。
- 非确定性或有副作用 Tool 不进入结果缓存。
- Target 变化产生新 Snapshot/Fingerprint。
- 相同 Blob 不合并不同 Provenance。
- Provider-specific 类型不泄漏到 Domain。
- Baseline 失效矩阵完整且有 Reason Code。
- Context 每个片段有选择理由。
- Replay 不执行真实工具且不产生正式证据。
- 默认二进制不依赖 Web、Server、MCP、多 Agent 或 Sandbox 产品层。

## 18. 变更控制

修改本架构需要：

1. 新建 ADR。
2. 描述问题、证据、方案、兼容性和迁移影响。
3. 明确是否改变 Event/Tool/Model/Artifact Schema。
4. 更新 `task.md` 和实施计划。
5. 发布新的冻结架构版本。
6. 不修改本文件来追溯伪造原决策。
