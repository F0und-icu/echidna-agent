# Cyber Agent 项目任务与精简架构

## 0. 文档定位

本文件是 `cyber-agent` 项目的当前权威任务文档。

参考材料：

- Codex 本地源码：`codex/`
- Codex 最新设计复核提交：`868c9edb0da913a5fc699a71664e65f44f6058b0`
- 分析文档：`Codex架构与漏洞挖掘Agent复用评估.md`
- 冻结架构：`docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` 至 `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_10.md` 的完整 additive chain
- 模型组合决策：`docs/adr/ADR-0002-capability-driven-primary-vision-models.md`
- 研究控制决策：`docs/adr/ADR-0009-stable-research-persona-domain-profile-and-phase.md`
- 不可变 Step/Tool Plan 决策：`docs/adr/ADR-0010-immutable-sampling-step-and-tool-plan.md`
- Context/Evidence/Compaction 决策：`docs/adr/ADR-0011-context-provenance-evidence-relations-and-compaction.md`
- 确定性检索/引导决策：`docs/adr/ADR-0012-deterministic-security-retrieval-and-evidence-gap-guidance.md`
- Prompt renderer 决策：`docs/adr/ADR-0013-versioned-prompt-renderer-boundary.md`
- ToolFailure/准确率可观测性决策：`docs/adr/ADR-0014-narrow-tool-failure-and-accuracy-observability.md`
- Responses reasoning continuation 决策：`docs/adr/ADR-0015-responses-reasoning-continuation.md`
- 冗余 encrypted reasoning sidecar 决策：`docs/adr/ADR-0016-redundant-encrypted-reasoning-sidecar.md`
- 交互运行连续性修复：`docs/adr/ADR-0017-interactive-runtime-continuity.md`
- 唯一 Provider 装配决策：`docs/adr/ADR-0006-single-provider-composition-root.md`
- 开发计划：`docs/development/IMPLEMENTATION_PLAN.md`
- 精简代码治理：`docs/development/LEAN_CODE_GOVERNANCE.md`
- 当前状态：`docs/development/STATUS.md`
- Git/版本/发布：`docs/development/GIT_VERSION_RELEASE.md`
- 开发环境：`docs/development/DEVELOPMENT_ENVIRONMENT.md`
- Codex 仓库约束：`AGENTS.md`
- GitHub 私有仓库：`F0und-icu/cyber-agent`

当前状态：

- 当前 F6a 为 `COMPLETE_LOCAL_LIVE_PASS`：全新 Terra xhigh 独立审计 APPROVE，完整 post-audit Gate（427 passed / 2 Live ignored，fmt/Clippy/build）、最终 Lean Review 均通过。两组正式 CLI DeepSeek Live 通过：32×16 原图返回两个 16×16 区域；2100×512 缩至 2048×499 后返回两个 1024×499 区域。尺寸/坐标/原始响应/审计一致，模型不确定性保留，不外推复杂截图定位准确性。Observation v2 保存 frame，v1 保持坐标系未知；ADR-0020 / Architecture v1.10 定义兼容边界。临时凭据副本已删除，原配置及 Codex 登录未动；未提交 F6a、未开始长上下文或 TUI，具体证据见 STATUS。

- F6a 随后获明确授权并已远端提交：实现 `82b42e1`，分支 `codex/f6a-vision-coordinate-contract`，Draft PR #69，基于尚未合并的 #68 分支，仅展示 F6a 增量。当前交付状态为 `REMOTE_SUBMITTED_PENDING_MERGE`；前述未提交说明是 Live 完成时的历史边界。两个 PR 均未合并，Actions 继续暂停；不启动下一 Slice。
- F4/F5 已远端提交：实现 `bd585ec`，短分支 `codex/f4-f5-cli-content-budget`，Draft PR #68，使用 `[skip ci]`。状态为 `REMOTE_SUBMITTED_PENDING_MERGE`；未合并 main、未创建 Tag/Release、不开始下一 Slice，Actions 继续暂停。下列 Live 完成记录中的 Git 未授权是当时测试边界，以本条及 STATUS 的当前交付状态为准。

- F5 已 `COMPLETE_LOCAL`：全新 Terra xhigh 独立只读审计 APPROVE，完整 post-audit Gate（421 passed / 2 Live ignored）、Lean Review 和 release 展示检查（8 passed）均通过。具体凭据来源获明确授权后，2026-09-11 开发版 debug/工具/Secondary Vision 与 release Chat 合成 Live 均 exit 0；2 Sessions、7 RequestAudits、5 model ToolCalls、6 Receipts、1 VisionObservation 的持久事实核验通过。Vision 左红右蓝正确，但区域尺寸估计错误，不能作为精确坐标或已验证证据；1M 配置与 1024000 输入预算路径通过，不代表实测满 1M 容量。临时凭据副本已删除，原配置及 Codex 登录状态未动。具体限制见 STATUS；未修改生产代码，Git/Actions/下一 Slice 未授权。

- F4 及其可读性改进已本地完成独立审计、完整离线 Gate 与 release 验证：所有构建默认在独立 stderr 连续流式展示 Provider 返回的明文思考，仅保留一次开头标识和明确的完成／中断提示；开发构建额外提供 `chat --debug` 结构化请求/响应诊断，标准 release 编译移除该开关和完整 wire 调试路径。具体边界见 ADR-0018/STATUS；未改写用户输入、未新增模型请求，改动尚未提交远端。

- 阶段：Implementation；M0 至 M5-S2 和 F1a-F1e/F2a 已远端封存；F3 已完成独立审计、完整离线 Gate、带明确限制的合成 Live 验收，并经 PR #67 / Squash Merge `5873c08` 远端封存，具体验证与 Git 状态见 STATUS。
- 当前代码基线已具备一个 AgentLoop、项目/会话/研究/审计事实、固定领域 Prompt Profiles、多模态组合、Replay/Eval、唯一 Provider Composition Root 和第一版 CLI Chat。
- Architecture v1.5/v1.6 已在本地冻结：Provider-visible completed reasoning 与 Conversation History 分离，并仅在有完整 `reasoning_text` 时忽略冗余 Adapter-local encrypted sidecar；architecture-manifest 远端封存仍未授权。
- Architecture v1.7 / ADR-0017 约束 F3 的逐阶段持久化、Research 前缀恢复、当前上下文接入、预算压缩与网络超时。本轮不实现非交互执行或 benchmark 适配，不自动授权 Git、Actions 或 Live。

## 1. 产品定义

Cyber Agent 是一个由单人使用和维护的、本地优先的漏洞挖掘专用 Agent。

它不是通用编程产品，不是团队协作平台，也不是 Codex 的缩小复刻。它只保留漏洞研究中真正需要的能力：

- 按项目隔离研究状态。
- 可持续的项目记忆和会话记忆。
- 可靠的工具调用循环。
- 轻量、结构化的研究台账，避免重复验证和研究漂移。
- 可编辑、可检查、可版本化的提示词。
- 一个稳定、务实、证据优先的漏洞研究人格；领域 Profile 与当前研究阶段独立组合。
- 长任务上下文管理。
- 项目快照、差量上下文、增量模型请求和可重建缓存。
- 文本和图片输入。
- 本地、可追溯的操作记录。
- 可交互 CLI，后续可选本地 Web 页面。

信任假设：用户、本机、项目文件和本地工具都属于同一个可信边界。Agent 自身的攻击面、恶意 Prompt/Tool Output、越权和本地多用户威胁不进入设计范围。

## 2. 硬性需求

### 2.1 必须实现

- 单进程、单主 Agent。
- Base Contract 只有一个 `Pragmatic Vulnerability Researcher` 身份，不提供多人格或角色 Agent。
- `DomainProfile`、`ResearchPhase` 和 `AgentModelProfile` 是正交轴，不创建组合角色。
- 一个主模型和至多一个可选视觉模型；视觉模型只提供受限推理能力，不拥有独立 AgentLoop、Memory、Research Ledger 或通用工具权限。
- 一个 Rust package 和一个可执行文件。
- 每个目标项目使用独立 `.cyber-agent/` 目录。
- 项目之间默认不共享会话、记忆、Prompt 和 Artifact。
- 默认 `FullAccess`，不启用沙箱，不强制工具确认。
- Shell、文件、网络和本地设备工具可直接调用。
- 模型可以连续调用工具，工具结果会进入后续采样。
- 每次模型采样捕获一个不可变 SamplingStepPlan；发送的 ToolSpecs、RequestAudit 和 Tool dispatch 使用同一 ResolvedToolPlan。
- 同一运行进程的本地 Session 内复用 Provider Session，可跨 Turn 追加；请求兼容时只发送增量 Input。
- 支持批量工具、长运行 Job 和基于 Resource Key 的并发调度。
- 支持 Project Snapshot/Diff，项目未变化时不重复扫描。
- 支持会话压缩、中断恢复和上下文预算。
- Context 具有可验证的来源 sidecar；Compaction 使用事实引用 Evidence Spine 与有损叙事，不复制第二套漏洞结论。
- 安全检索和证据缺口/实验引导保持本地、确定性、可解释、Shadow-first，未证明收益前不改变模型输入或自动执行。
- 所有审计数据只写本地。
- 不上报 telemetry、analytics、feedback、crash dump 或调试日志。
- 不需要云 Server、远程任务或账号系统。

### 2.2 专用于漏洞研究

- 项目记忆保留 Target、Version、Environment、Attack Surface、已验证事实、未验证假设、失败路径和下一步。
- Research Ledger 使用稳定 ID 管理 Hypothesis、Experiment、Finding Record 和 Next Action，但不强制复杂状态迁移。
- Prompt 中要求区分静态候选、可复现问题和已证明影响，但不在 Core 中实现繁重的 Finding 状态机。
- 大型命令输出、安装包、抓包、日志、截图和二进制文件以 Artifact 引用进入上下文。
- 提供固定 `source`、`web`、`mobile`、`binary` DomainProfile；项目特殊约束进入 Project Prompt，Firmware 在有真实需求前不提供 Profile。
- 提供固定 `mapping`、`hypothesis`、`experiment`、`verification`、`evidence_curation` ResearchPhase，只改变当前目标、Prompt 和 Tool Exposure，不替代 Research Ledger。
- 漏洞研究工具主要继续使用现有 CLI 和脚本，不为每个工具建立 Rust 封装层。

## 3. 明确不做

- 多 Agent 编排。
- 团队、租户、角色和权限管理。
- 云同步、云任务、远程控制和 App Server。
- Marketplace、通用插件商店和 Connector 产品层。
- 企业登录、计费、用量上报和反馈系统。
- Guardian、复杂 Policy Engine 和授权工作流。
- 跨平台沙箱全集。
- Agent 安全威胁建模、Prompt Injection 防护、危险命令分级和本地 Web 鉴权。
- 通用 IDE、完整 TUI 和代码审查产品线。
- 自建模型网关、模型调度 Server 和远程记忆服务。
- 为了“模块化”而拆分大量 crate。
- 多人格/角色目录、Domain × Phase 组合角色、通用 Prompt/Phase Registry 和研究 Workflow Engine。
- PromptDialect/renderer Registry、Tool Router 平台、selector framework、默认向量/图数据库、远程 embedding/reranker/judge 和自动实验生成器。

## 4. 总体架构

    Interactive CLI
          │
          ▼
    Agent Runtime
      ├── Primary Provider Session
      │   ├── ModelInputIR / Provider Encoder
      │   ├── Streaming / Retry
      │   ├── previous_response_id
      │   └── Prompt Cache Key
      ├── Vision Route Resolver
      │   ├── auto / primary_direct
      │   ├── secondary_tool / disabled
      │   └── Optional VisionAnalysisService
      ├── Project Manager / Snapshot
      ├── Context Planner
      │   ├── Fragment Cache
      │   ├── Snapshot / Diff
      │   ├── generic-v1 Renderer / ContextDeliveryMap
      │   └── Deterministic Security Retrieval (Shadow first)
      ├── Memory Manager
      ├── Research Ledger / Provenance
      │   ├── TargetSnapshot / ExperimentCapsule
      │   ├── EvidenceLink / Evidence Gap
      │   └── Observation / TargetDelta View
      ├── Research Control Plane
      │   ├── Stable Base Contract
      │   ├── DomainProfile
      │   └── ResearchPhase
      ├── Prompt Composition
      ├── Tool Runtime
      │   ├── Immutable Catalog / ResolvedToolPlan
      │   ├── Direct / Turn-local Deferred Tools
      │   ├── Batch / Resource Locks
      │   ├── Effect / Intent / Receipt
      │   └── Jobs / PTY
      ├── Replay / Eval Adapters (P1)
      └── Local Audit Writer
          │
          ├── .cyber-agent/memory/
          ├── .cyber-agent/research.jsonl
          ├── .cyber-agent/sessions/
          ├── .cyber-agent/artifacts/
          └── .cyber-agent/cache/

架构原则：

- 一个二进制文件。
- 一个主循环。
- 一个项目一份状态。
- 一套人类可直接阅读和修改的本地文件。
- 先用内部 Rust module 隔离职责，真的出现独立发布或重依赖需求时才拆 crate。
- 没有通用 Hook Bus；只保留主循环中明确的前后处理点。
- 没有人格、Prompt Stage、DomainProfile 或 ResearchPhase 动态 Registry；只保留固定资源和一个组合路径。
- 没有 Prompt renderer/Tool selector/Context source 动态 Registry；一个当前实现使用显式版本和固定纯函数。
- 权威数据使用 Markdown/JSONL/Artifact；性能缓存可随时删除和重建。
- 效率优化优先减少模型往返、重复 Token、重复扫描和重复工具调用，不通过增加多 Agent 解决。

## 5. 单 Agent 主循环

    receive user input
      ↓
    load TurnContext and ProviderSession
      ↓
    apply queued DomainProfile / ResearchPhase / AgentModelProfile at Turn boundary
      ↓
    append local audit event
      ↓
    refresh cheap Project/Runtime fingerprints
      ↓
    capture TurnConfigurationSnapshot and StepSnapshot
      ↓
    update Research State and Project Snapshot diff
      ↓
    resolve image route from VisionMode and explicit ModelCapabilities
      ↓
    when secondary_tool, create scoped VisionObservation before primary sampling
      ↓
    resolve immutable SamplingStepPlan + ResolvedToolPlan
      ↓
    render generic-v1 ContextPlan + ContextDeliveryMap
      ↓
    persist RequestAudit, build ModelInputIR and choose full or incremental delivery
      ↓
    stream text / reasoning items / tool calls
      ↓
    tool call?
      ├── no  → persist response → update session memory → wait for input
      └── yes
           ↓
         validate ToolCall against the same ResolvedToolPlan
           ↓
         resolve ToolExecutionPlan and persist execution intent
           ↓
         schedule by Resource Key
           ↓
         execute one or more tools with FullAccess
           ↓
         persist ExecutionReceipt, output or Artifact/Observation reference
           ↓
         update Project Snapshot / Research Ledger / Job state
           ↓
         append ToolResult to history
           ↓
         update Turn-local DeferredActivationSet when tool_search matches
           ↓
         capture a new immutable plan and append only changed context fragments
           ↓
         sample model again

主循环只负责：

- 会话状态。
- Prompt 和上下文组装。
- 稳定 Base、DomainProfile、ResearchPhase 与 Tool Exposure 的固定组合。
- 每个 Step 的 Context/Tool/model/capability 计划捕获和一致性。
- 模型流式输出。
- 工具调用与结果回传。
- 取消、重试和恢复。
- 本地审计事件。
- 轻量性能计数。

不在主循环中加入通用工作流编排、多 Agent 调度或复杂漏洞状态机。

### 5.1 TurnContext

一次用户输入开始时创建，在该 Turn 的多次模型采样中保持稳定：

- Turn ID。
- Model Profile。
- Stable Base/Personal/Project/Domain/ResearchPhase Prompt Version。
- Provider Session。
- Context Budget。
- Tool Exposure Baseline。

### 5.2 StepSnapshot 与 SamplingStepPlan

每次模型采样前快速重新捕获：

- Project Fingerprint。
- Runtime Environment Fingerprint。
- Research Ledger Version。
- 当前 TargetSnapshot ID 和 Toolchain Fingerprint。
- immutable Tool catalog、当前 Turn DeferredActivationSet 和 resolved plan identity。
- 设备、PTY 和后台 Job 状态。
- 本 Turn 已修改文件和新建 Artifact。

StepSnapshot 不做全项目重扫描。它先比较便宜的 mtime、size、Git HEAD/dirty state 和工具状态，只在指纹变化时刷新对应 Snapshot。

每次模型采样再捕获不可变 SamplingStepPlan，绑定 TurnConfigurationSnapshot、ContextPlan、ResolvedToolPlan、AgentModelProfile/Capability Mapping、RequestRequirements 和 `generic-v1` renderer。该 Plan 同时决定发送 ToolSpecs、RequestAudit、ToolCall validation/dispatch、Receipt binding 和 Baseline admission；捕获后不受 queued 配置或后续 `tool_search` 修改。

`tool_search` 匹配只更新 typed DeferredActivationSet，使当前 Turn 的下一 Step 重算 Plan。Activation 在 Turn 结束和 Resume 后清空；Hidden/Unavailable Tool 永不进入集合。

### 5.3 固定处理点

不实现通用 Hook Runtime，但主循环保留固定内部处理点：

- `turn_boundary_configuration`：应用排队的 DomainProfile、ResearchPhase 和 AgentModelProfile，组合当前 Tool Exposure；模型自由文本不能触发切换。
- `before_model_request`：刷新 Snapshot，捕获 SamplingStepPlan，组装 ContextPlan/DecisionManifest/ContextDeliveryMap 和 ModelInputIR，持久化 RequestAudit 后决定全量/增量请求。
- `before_tool_execution`：使用产生该 ToolCall 的 ResolvedToolPlan 验证 Tool Schema/dispatch，返回结构化执行前错误或建立 ToolExecutionPlan；不回查新 Plan，不自动重试。
- `after_tool_result`：持久化 ExecutionReceipt/Artifact Lineage，更新 Observation、Research Ledger、Project Diff 和 Job 状态。
- `turn_end`：更新 Session Summary、Project Memory 和效率指标。

它们是直接的 Rust 函数/中间件，不包含动态注册、外部脚本协议或插件生命周期。

## 6. 项目隔离

### 6.1 项目发现

- `cyber-agent init` 在当前目录创建 `.cyber-agent/project.toml`。
- 运行时从当前目录向上查找 `.cyber-agent/`。
- 可使用 `--project <path>` 显式选择项目。
- Project ID 和 canonical root path 一起标识项目。

### 6.2 项目目录

    <target-project>/
    └── .cyber-agent/
        ├── project.toml
        ├── tools.toml
        ├── research.jsonl
        ├── memory/
        │   ├── core.md
        │   ├── notes/
        │   ├── decisions.md
        │   └── failed-paths.md
        ├── prompts/
        │   └── project.md
        ├── sessions/
        │   └── <session-id>/
        │       ├── events.jsonl
        │       ├── summary.md
        │       └── request-snapshots/
        ├── artifacts/
        │   └── blobs/
        │       └── sha256/
        └── cache/
            ├── project-snapshot.json
            ├── file-manifest.json
            ├── tool-capabilities.json
            ├── context-fragments/
            └── retrieval-index/

隔离语义：

- 项目隔离的是模型上下文、记忆、Prompt、会话和审计数据。
- `FullAccess` 下工具仍可访问系统其他路径、网络和设备；项目隔离不是沙箱。
- 除非用户显式操作，一个项目的记忆不注入另一个项目。
- 全局只保存个人偏好、Provider 配置和默认 Prompt，路径使用操作系统标准 config directory。
- `cache/` 中的任何内容都不是事实源，删除后必须能从项目文件、Memory、Research Ledger 和 Session Event 重建。

### 6.3 数据所有权和事实源

| 数据 | 权威来源 | 派生数据 |
|---|---|---|
| 项目配置 | `project.toml`、`tools.toml`、Project Prompt；固定 Domain/Phase 资源来自版本化程序资源 | 解析后的运行时配置 |
| 对话、模型请求和工具审计 | `sessions/<id>/events.jsonl` | Prompt History、Session Summary、Metrics |
| TargetSnapshot/Hypothesis/Experiment/Finding/Next Action | `research.jsonl` | ResearchState、ResearchWorldState Section、TargetDelta |
| 人工/模型整理的长期知识 | `memory/*.md` | Memory Retrieval Index、注入片段 |
| Artifact 身份、来源和派生关系 | Session/Research 中的 `ArtifactCreated/ArtifactDerived` Event | Artifact Lineage Graph |
| 大型原始输出、文件和去重后的请求内容字节 | content-addressed Blob Store | 缩略图、摘要、显示副本 |
| Project Snapshot、File Manifest、Tool Capabilities | 从项目和工具重建 | `cache/` 中的性能快照 |
| ContextPlan/Fragment Cache | 从 Prompt、Memory、Research、History 重建 | `cache/context-fragments/` |

不允许 Session Summary 覆盖 Research Ledger，不允许 Cache 覆盖 Memory，不允许 Prompt 文本成为工具执行状态的唯一记录。

### 6.4 通用事件信封

Session Event 和 Research Event 共用最小信封：

    EventEnvelope<T> {
        schema_family
        schema_version
        event_id
        stream_id
        sequence
        occurred_at
        event_type
        payload: T
    }

规则：

- `sequence` 在单个 stream 内严格递增；Session 和 Research 使用不同 stream。
- 每个 stream 使用一个 Event Writer 串行分配 sequence 和追加 JSONL；并行 Tool/Job 不直接同时写文件。
- 进入可写 `chat` 前获取项目级 OS advisory writer lock；同一项目第二个写进程必须明确失败或只读打开，不能依赖“锁文件存在”判断存活。
- `event_id` 用于重放去重，同 ID 事件不应被 Reducer 应用两次。
- 新格式只通过新的 `schema_family + schema_version` 发布；所有已经对本地项目发布过的版本必须保留连续 Upcaster 链，不能升级两次后就放弃读取旧项目。
- Upcaster 只在读取时将旧 Event 转成当前内存表示，不原地重写原始 JSONL；每个 `vN -> vN+1` 都有固定 fixture 和纯函数测试。
- 遇到缺失 Upcaster 或会影响 State 的未知 Event Type 时进入只读诊断模式，不得静默忽略后继续写入。
- 如果用户需要永久升级文件，使用显式 export/migrate 生成新 Stream，并记录源 Stream Hash、目标 Schema 和迁移工具版本；原 Stream 默认保留。
- 先向 JSONL 追加事件，成功后再将事件应用到内存 State。
- 模型流式 token 不逐 token 写 Event；在完整 Response Item 形成后再追加，减少磁盘写入。
- Session Stream 和 Research Stream 不伪造跨文件事务；先持久化 Research Event，再在 Session Event 中写引用。中断后孤立的有效 Research Event 仍是权威记录。
- 恢复时从最新可用派生 Snapshot 开始，从 `last_sequence + 1` 继续重放；Snapshot 不可用时从头重放。
- 打开 Event Stream 时只接受完整换行且可解析的尾部；若最后一行不完整或无效，先把原始尾字节存入恢复 Artifact，再截断到最后一个有效边界，并以 `StreamRecovered` 事件从最后有效 sequence 继续。
- 尾部恢复不得跳过中间损坏后继续猜测事件边界；非尾部损坏必须停止可写模式并给出精确 offset，避免静默丢事件。

### 6.5 Reducer 和恢复

Reducer 是 `domain` 中的纯函数：

    raw event -> validate envelope -> chained upcasters -> current event -> reducer

    reduce_session(SessionState, SessionEvent) -> SessionState
    reduce_research(ResearchState, ResearchEvent) -> ResearchState

- Reducer 不读文件、不调模型、不执行工具、不获取当前时间。
- Upcaster 同样是纯函数，不访问当前项目状态，不补写基于“现在”的推测字段。
- 对同一有序事件流必须生成相同 State。
- 事件先持久化再 apply，避免内存状态已更新但本地事件缺失。
- Session Event 可引用 Research Event ID 和 Artifact ID，但不复制另一事实源的完整 Payload。
- `summary.md`、ResearchWorldState、Metrics 和检索索引都是 Reducer/生成器的派生视图。

## 7. 记忆管理

### 7.1 三层记忆

1. Working Memory
   - 当前上下文中的近期消息和工具结果。
   - 超出预算后可被压缩。

2. Session Memory
   - 当前研究会话的 `summary.md`。
   - 自动更新，用于 resume 和 compaction。

3. Project Memory
   - 跨会话保留的稳定事实、决策、失败路径和下一步。
   - 以 Markdown 保存，可直接用编辑器维护。

全局 Personal Memory 是可选能力，只保存用户偏好和通用工作方式，不自动吸收项目漏洞细节。

### 7.2 项目记忆结构

`core.md` 使用稳定 Section，并且有严格大小上限：

- Project Goal
- Target / Version / TargetSnapshot ID
- Environment
- Confirmed Facts
- Active Hypotheses
- Attack Surface
- Important Artifacts
- Blockers
- Next Actions

`notes/` 保存按主题拆分的长期笔记，例如某个组件、协议、包名、函数、攻击面或设备。它们不默认全量注入 Prompt。

`failed-paths.md` 记录：

- 已否定的假设。
- 不可达入口。
- 无效 Payload。
- 版本、设备和账号前提。
- 否定证据和 Artifact 引用。

### 7.3 记忆操作

- `/remember <text>`：写入项目记忆。
- `/forget <query>`：删除或修正记忆。
- `/memory`：查看当前已注入记忆。
- `/memory edit`：用默认编辑器打开记忆文件。
- `/memory diff`：显示本会话对长期记忆的改动。
- `/memory rebuild`：从本地会话、Research Ledger 和审计事件生成 `memory/rebuild-candidate.md`，不直接覆盖权威 Memory。

模型可以通过 `memory_write` 工具更新 Project Memory，但每次改动都写入本地审计事件，且可以回看 diff。跨项目的 Global Memory 只允许用户显式写入。

Memory 文件使用临时文件 + atomic rename 写入。`MemoryChanged` 事件记录旧 Hash、新 Hash、Patch 以及旧/新内容寻址 Blob 引用，但 Markdown 文件本身仍是权威内容。

Memory Manager 在项目打开和 Turn 开始时比较当前文件 Hash 与最近已审计 Revision。用户通过外部编辑器直接修改后，在下次发现时保存新 Revision Blob，追加 `MemoryExternalChangeDetected` 和 Diff；Watcher 只能提前触发检查，不能作为变化事实源。

### 7.4 记忆检索

上下文不再无条件注入全部 Project Memory：

- `core.md` 的短小核心事实始终注入。
- `notes/`、`decisions.md`、`failed-paths.md`、Research Records 和 Artifact/Observation metadata 使用独立 candidate eligibility，先排除跨 Project、错误 Target/Version 和不可用 Artifact。
- 固定版本化 pipeline 为 exact security identifiers → fielded BM25 → CJK/ASCII character n-gram → RRF → authority/freshness tiered rerank → stable ID tie-break。
- Query views 只由当前输入、精确标识符、DomainProfile/ResearchPhase、active Hypothesis、TargetSnapshot 和 typed evidence gap 确定性生成，不调用模型扩写 query。
- 检索结果保留来源路径/Record、Section、Hash、exact matches、component ranks、authority/freshness disposition、inclusion/exclusion reason 和 Context cost。
- 首个实现只用 bounded scan/in-memory derived index 并运行 Shadow，不改变模型输入。promotion 需要 controlled acceptable-reference fixtures 证明 recall/precision、重复读取或 Token 收益。
- 只有实测延迟证据、可删除重建路径和独立依赖决策后才考虑 SQLite FTS；默认不使用向量数据库、remote embedding/reranker 或 model judge。
- 每次模型请求记录注入了哪些记忆片段及其 Token 占用。

## 8. Research Ledger 与上下文管理

### 8.1 轻量 Research Ledger

`.cyber-agent/research.jsonl` 是可追加、可手工阅读的研究台账。它解决长任务中的去重、优先级、证据引用和恢复问题，不实现复杂工作流。

核心记录：

    HypothesisRecord {
        id
        title
        target
        attack_surface
        vulnerability_class
        entry_point
        sensitive_sink
        dedup_key
        rationale
        status: active | testing | supported | rejected | blocked
        priority
        evidence_refs
        last_action
        next_action
    }

    ExperimentRecord {
        id
        hypothesis_id
        target_snapshot_id
        capsule_id
        goal
        tool_calls
        environment
        action_fingerprint
        result: pending | positive | negative | inconclusive
        artifact_refs
        notes
    }

    FindingRecord {
        id
        title
        status: candidate | reproduced | confirmed | discarded
        dedup_key
        target_version
        target_snapshot_ids
        prerequisites
        impact
        evidence_refs
        negative_refs
    }

    NextActionRecord {
        id
        research_id
        priority
        action
        status: pending | running | done | blocked
    }

规则：

- 所有记录使用稳定 ID，Prompt、Memory、Tool Result 和 Artifact 通过 ID 引用。
- 状态更新写入新事件，不静默覆盖历史。
- Core 不阻止状态转移，不要求安全审批或自动 Validator。
- `confirmed` 只是研究语义，Base Prompt 要求其有可引用的实验和证据。
- 同标题、同 Target、同 Sink 或同 Artifact 的记录在新建前先做去重检查。

M4-S3e1 通过 additive payload 将证据角色变为 `EvidenceLink { relation, evidence }`。固定 relation 是 supports、contradicts、positive_control、negative_control、environment_control、reachability_proof、prerequisite_proof 和 impact_proof。它只能由显式 `research_update` 创建；Prompt、Summary、Replay/Eval/Vision 和模型 confidence 都不能推断或升级关系。

历史 `evidence_refs` 保守 Upcast 为 supports，`negative_refs` 为 contradicts，不把旧数据猜成 control。Gap projection 只读 typed fields/links，并明确区分 unknown 与 missing。

#### 确定性去重

Hypothesis/Finding 的 `dedup_key` 使用稳定字段组合生成：

    normalize(target_id)
    + normalize(vulnerability_class)
    + normalize(entry_point)
    + normalize(sensitive_sink)

- `normalize` 只执行字符大小写、空白、路径/符号规范化和已配置 Alias 替换，不调用模型。
- 精确 `dedup_key` 命中时，`research_update` 返回现有 Record，并要求调用方选择追加 Evidence、保留独立记录或显式合并。
- 标题或语义相似只作为候选提示；Core 不基于模型相似度自动合并。
- 合并必须写入 `RecordsMerged { from_id, into_id, reason }` 事件，原 Record 保留 `merged_into` 引用，不从历史中删除。

Experiment 的 `action_fingerprint` 使用：

    hypothesis_id
    + normalized tool/command and semantic arguments
    + target_snapshot_id
    + tool/extractor version
    + relevant input artifact hashes

- 同 `action_fingerprint` 的已完成 Experiment 在新建前返回重复提示和原结果。
- 当 Target Version、Environment、Payload 或输入 Artifact 发生变化时，会生成新 fingerprint，允许独立复测。
- 去重提示不阻止 FullAccess 执行；用户或 Agent 仍可显式要求 rerun，并记录 rerun reason。

### 8.2 可复现研究内核

#### TargetSnapshot 和 ExperimentCapsule

`ProjectSnapshot` 回答“本地项目文件是否变化”，`TargetSnapshotRecord` 回答“漏洞结论针对的真实目标是什么”。二者不能混用。

    TargetSnapshotRecord {
        id
        target_id
        kind: source | web | mobile | binary | firmware
        locator
        version
        build_id
        target_artifact_ids
        environment_fingerprint
        device_fingerprint
        snapshot_fingerprint
        captured_at
    }

    ExperimentCapsuleRecord {
        id
        experiment_id
        target_snapshot_id
        normalized_action
        execution_ids
        input_artifact_ids
        environment_delta
        account_prerequisites
        toolchain_fingerprint
        execution_receipt_ids
        output_artifact_ids
    }

规则：

- `snapshot_fingerprint` 只包含目标身份、Version/Build、目标 Artifact、设备和关键运行环境，不包含 captured_at、分析工具版本或备注。
- 相同 fingerprint 复用已有 TargetSnapshot；任一语义字段变化时创建新 ID，已创建 Snapshot 不静默修改。
- 分析工具版本属于 ExperimentCapsule/ExecutionReceipt，不因更换 JADX、Frida 或扫描器而伪造目标版本变化。
- Experiment 必须引用一个 TargetSnapshot；无法确定版本或环境时显式记录 `unknown`，不能沿用最近一次值。
- Capsule 只保存复现实验必需的稳定参数和引用，不复制完整 stdout/stderr、抓包或二进制字节。
- `reproduced/confirmed` Finding 必须引用实际成立的 TargetSnapshot；跨版本结论需要多个 Snapshot 或明确的 Version Range 证据。
- TargetSnapshot 和 Capsule 进入 `research.jsonl`，不新增数据库或独立工作流服务。

#### Artifact Lineage

Blob 和 Artifact 身份分离：相同字节只保存一次，但在不同实验中产生时保留不同来源记录。

    ArtifactRecord {
        artifact_id
        blob_id: sha256
        media_type
        size
        created_by_event_id
        produced_by_tool_call_id
        target_snapshot_id
        parent_artifact_ids
        transform
        toolchain_fingerprint
    }

- `blob_id` 标识内容；`artifact_id` 标识一次有研究语义的引用和来源，不能因为 Hash 相同就合并 Provenance。
- 原 APK、解包目录清单、反编译源码、裁剪日志、缩放截图和报告附件通过 `parent_artifact_ids + transform` 形成派生链。
- Artifact Lineage Graph 是从 Event 和 ArtifactRecord 生成的派生视图，不引入图数据库。
- 删除无引用 Blob 前必须证明没有 Session、Research、Memory、RequestAuditManifest 或 ArtifactRecord 引用；MVP 不自动做破坏性 GC。

#### ObservationEnvelope 和版本差分

领域工具可以在原始输出之外返回可选的结构化观察：

    ObservationEnvelope {
        schema_version
        observation_id
        kind
        target_snapshot_id
        location
        subject
        entry_point
        source
        sink
        prerequisites
        confidence
        evidence_refs
        produced_by_tool_call_id
        extractor_id
        extractor_version
    }

- Observation 是带来源的工具观察，不自动成为 Hypothesis、Finding 或已验证事实。
- 原始工具输出仍由 Tool Event/Artifact 保存；Observation Index 只是可删除重建的检索缓存。
- 只有显式 `research_update` 才把 Observation 引用提升为 Research Record，模型置信度不能自动完成提升。
- `TargetDelta` 从两个 TargetSnapshot 及其 Observation 派生，可比较组件、路由、权限、API、符号、Source/Sink 和攻击面变化。
- 首期通过外部工具或脚本产生 Observation，不在 Core 内重写 JADX、Frida、Nmap、Tree-sitter、反编译器或扫描器。

### 8.3 ResearchWorldState

不移植 Codex 的完整 World State，只实现与漏洞研究相关的 Section：

- `target`
- `target_snapshot`
- `target_delta`
- `environment`
- `project_snapshot`
- `attack_surface`
- `active_hypotheses`
- `recent_experiments`
- `findings`
- `failed_paths`
- `active_jobs`
- `next_actions`

每个 Section 必须提供：

- 稳定 Section ID。
- 结构化 Snapshot。
- Snapshot Hash。
- Full Renderer。
- Diff Renderer。
- Token 上限。

这个设计只借鉴 Codex World State 的 Snapshot/Diff 机制，不引入权限、Plugin、Collaboration、Realtime 等通用 Section。

### 8.4 ContextFragment 和 ContextPlan

Context Builder 改为 Context Planner。每个输入片段实现：

    ContextFragment {
        id
        slot
        origin
        semantic_class
        authority_level
        retention_class
        freshness_identity
        references
        priority
        content_hash
        token_estimate
        cache_policy
        content_items
    }

最终产生：

    ContextPlan {
        base_instructions
        input_items
        tool_specs
        parallel_tool_calls
        output_schema
        prompt_cache_key
        context_fingerprint
        fragment_manifest
        context_decision_manifest
        context_delivery_map
        prompt_renderer: generic-v1
    }

    ContextDecisionManifest {
        context_fingerprint
        budget
        candidates: [
            fragment_id
            slot
            origin
            content_hash
            token_estimate
            decision: include | truncate | artifact_ref | exclude
            reason_code
            baseline_relation
        ]
    }

`ContextDecisionManifest` 记录选择过程而不是复制片段正文：

- 每个候选片段必须有稳定 Reason Code，例如 `REQUIRED_RECENT_INPUT`、`ACTIVE_RESEARCH`、`RETRIEVAL_MATCH`、`BUDGET_EXCLUDED`、`STALE_SNAPSHOT`。
- `/context` 可以解释包含、裁剪和排除原因，而不只展示最终结果。
- 离线 Eval 可以针对同一历史 Turn 重新运行 Context Planner，比较 Token、覆盖率和决策变化；重新编译结果是 Eval 派生数据，不改写原请求审计。
- Selection 算法或 Token Estimator 变化必须进入 Context Planner Version 和 Fingerprint。

`ContextSlot` 只负责 Base/Personal/Project/Domain/Phase/Research/Project Snapshot/Memory/Summary/History/User/Turn-local 的固定位置。Origin、SemanticClass、AuthorityLevel、RetentionClass 和 FreshnessIdentity 不得影响排序。M4-S3a1 只迁移最小 Slot/Origin 并保持现有 bytes；完整 provenance 在 M4-S3e2 实现。

每个 Fragment 具有稳定 ContextFragmentId，每个 fragment-local 条目具有确定性 ContextSubitemId；exact bytes 另有 content hash。Provider 可选 DeliveryItemId 只在 Adapter 内为一次 prepared request 的 transport retry 使用，不能进入 Domain/Research identity，也不能跨 Primary/Vision。

ContextPlanner 使用唯一 `generic-v1` renderer，并产生不进入模型文本的 `ContextDeliveryMap` sidecar：instruction byte span 和 ContentItem final position 都必须恰好映射到 Fragment/Subitem、content hash、transform 和 provenance；renderer separator 是唯一例外。Renderer identity 进入 Context/Prompt fingerprint、RequestAudit、Baseline 和 Replay。没有第二个真实 renderer 和 Eval 证据前，不增加 PromptDialect Trait/Registry/Config。

与 Codex 一样，模型请求保持三个独立通道：

- Base Instructions。
- Conversation/Context Input Items。
- Tool Specs。

工具 Schema 不拼接进普通 Prompt 文本。

### 8.5 固定组装顺序

1. Base Contract（`Pragmatic Vulnerability Researcher`）。
2. Personal Preferences。
3. Project Prompt。
4. 当前 DomainProfile Prompt。
5. 当前 ResearchPhase Prompt。
6. ResearchWorldState Full/Diff。
7. Project Snapshot Full/Diff。
8. Project Memory Core。
9. 按需检索的 Memory Fragments。
10. Session Summary。
11. 最近会话历史。
12. 当前用户输入。
13. Turn-local Tool/Job Update。

稳定内容始终放在前面，动态内容放在后面，便于 Prompt Cache 和调试。

上述顺序定义 Full Request 和 Baseline 重建，不代表每个 Turn 都重新渲染并替换全部 Items。Incremental Request 不重排旧 Items，只在已确认的 Baseline 后按实际因果顺序追加 Delta。

- 活跃 Baseline 已经包含完整历史时，`summary.md` 的本地更新不自动替换 Provider 上下文；Session Summary 只在首次请求、Resume、Compaction 或全量回退时进入 Full Request。
- Research、Project Snapshot 或 Project Memory 的变化优先渲染为带 Section ID、旧/新 Hash 和来源的 Delta Item；Turn 开始前发现的变化排在新 User Input 前，Tool 执行产生的变化排在对应 Tool Result 后。
- 如果变化无法用追加项无歧义表达，或需要删除、重排、替换旧语义，Context Planner 标记 `not_append_only`，Provider Session 清空 Baseline 并发送 Full Request。
- Context Planner 维护 Full Logical Manifest 和相对当前 Baseline 的 Append-only Candidate；未变 Fragment 只引用缓存内容和 Hash，不重新渲染。

### 8.6 上下文预算

Context Builder 必须：

- 根据模型 context window 计算可用预算。
- F5 默认预留为 `min(window/8, 8192)` 回复和 `min(window/8, 16384)` 工具循环；1,048,576 窗口的输入额度为 1,024,000。Chat、Compaction、Vision 复用同一 ContextPlanner 和版本化策略，不设隐藏 32K 工作上限。窗口来自显式 CapabilityMapping，不按模型名称推断，也不代表服务端必然支持该容量。
- 为模型回复和工具循环保留空间。
- 对工具输出进行头尾保留和结构化截断。
- 将大型内容保存为 Artifact，Prompt 中只传摘要、路径和 Hash。
- 优先保留用户最新输入、未完成工具对、项目记忆和 Session Summary。
- 删除孤立 Tool Result，并为缺失结果的 Tool Call 生成稳定的 `aborted` 占位。
- 优先删除低相关 Memory Fragment 和已过期 Project Snapshot，不删除活跃 Research Record。

### 8.7 Snapshot/Diff 和片段缓存

- 首次请求、resume 后首次请求和 compaction 后首次真实请求注入必要 Full Snapshot。
- 同一 Turn 后续采样只追加工具结果和发生变化的 Section Diff。
- 同一进程的下一 Turn 在 Baseline 仍兼容时，只追加新的用户 Input 和变化片段；任何中间历史替换仍按失效矩阵回退全量请求。
- Fragment Cache 命中不等于 Provider Baseline 兼容；前者减少本地渲染，后者必须通过完整失效矩阵判断。
- Fragment Cache Key 至少包含 Fragment ID、内容 Hash、Prompt Version、Model Capability 和预算策略。
- 所有缓存都是可选优化；缓存损坏或版本不兼容时删除重建。

Project Snapshot 使用两级失效算法，不在每个 Step 全量 Hash 所有文件。

#### 第一级：便宜 ProjectFingerprint

Git 项目：

- canonical project root。
- Git HEAD/OID。
- Git index 元数据或等价快速指纹。
- `git status --porcelain=v2 -z` 的稳定 Hash。
- untracked path 清单摘要。
- 关键 Manifest 的 path/size/mtime。

非 Git 项目：

- canonical project root。
- 排序后的 relative path + file type + size + mtime 清单摘要。
- 关键 Manifest 元数据。
- 已配置的忽略目录和大文件阈值。

StepSnapshot 只计算这一级指纹。如果它与缓存一致，直接复用 Project Snapshot、File Manifest 和已有文件 Hash。

#### 第二级：按需内容验证

- 只对新增、修改、当前工具读取或 Research Ledger 引用的文件计算内容 Hash。
- 性能缓存和文件指纹使用 BLAKE3。
- 需要长期引用、报告交付或跨工具校验的 Artifact 使用 SHA-256。
- 文件缓存条目保存 canonical path、size、mtime、BLAKE3 和最后读取范围。
- 已删除文件必须从 File Manifest 和索引中移除。
- 元数据冲突、Git 状态无法解析或缓存 Schema 不兼容时，执行可解释的全量重建。

#### 缓存语义

- Project Snapshot 包含 ProjectFingerprint、文件清单摘要、语言/技术栈、关键 Manifest、已选 Target、工具能力和 `last_research_sequence`。
- 每个缓存文件保存 `cache_schema_version`、Project ID、Source Fingerprint 和 created_at。
- 快照未变化时，`inspect_project` 直接返回缓存和指纹，不重新遍历文件内容。
- 文件系统 Watcher 只能作为运行期失效提示，不是恢复后的事实源。
- `/snapshot` 必须显示本次是 Hit、Incremental Refresh 还是 Full Rebuild，以及触发原因。

### 8.8 Compaction

当前正式 Chat 仅由操作者显式 `/compact` 触发；预算阈值只提示：

1. 生成结构化 Session Summary。
2. 保留最近消息和完整 Tool Call/Result 对。
3. 使用同一个 ContextPlanner 检查下一 Turn 探测输入的预算与实际缩减效果；按完整配对后缀缩减保留量，无收益或无法容纳则在写入前拒绝。
4. 先写入 `Compaction` 事件，再用其 Summary/保留后缀更新运行时。
5. 原始会话仍保留在本地 `events.jsonl`，不删除。

Session Summary v2 分离：

- `EvidenceSpine`：TargetSnapshot、active Hypothesis、Experiment、Finding、NextAction、Artifact、ExecutionReceipt 引用，以及绑定的 Research stream identity + watermark/hash。
- `NarrativeSummary`：当前目标、简短进展、未解决问题和下一步说明；它有损、bounded 且不是事实。

Compaction Event 只证明本次压缩已发生。Evidence Spine 是有界导航引用子集，Narrative 只帮助续写；两者都不能创建或升级 Research claim。append 前验证每个引用的 Project/stream/type/watermark。Research 仅追加时，先严格验证旧 Summary 的完整历史前缀 hash/引用，再以当前目标重建新 Compaction 并写入，最后 Resume；缺失、跨项目、篡改和类型不匹配仍失败。历史 Summary v1 保持可读，不把旧字符串升级成 typed evidence。预算预检不能保证任意长度的未来输入都可容纳。

## 9. Prompt 管理与优化

### 9.1 Prompt 来源

- 内置 Base Contract：保持短小，固定唯一 `Pragmatic Vulnerability Researcher` 身份、漏洞研究原则、工具协议和输出格式。
- 全局 Personal Prompt：用户的偏好和工作方式。
- Project Prompt：当前目标的特殊约束和工作流。
- DomainProfile Prompt：固定 source、web、mobile、binary 领域研究指引。
- ResearchPhase Prompt：固定 mapping、hypothesis、experiment、verification、evidence_curation 当前目标指引。

固定 Base/Domain/Phase Prompt 保存为独立 Markdown 资源并以 `include_str!` 进入单一二进制；Personal/Project Prompt 由 Config v1 的可选 `[prompts] personal_file/project_file` 显式选择，路径相对配置文件解析。正式 Chat 在采样前读取配置的 UTF-8 普通文件；缺失的显式文件报错。没有目录搜索、自动发现、Persona/Prompt Registry；doctor 不读取 Prompt 内容。Project Core Memory 使用现有 Memory owner 读取并通过 ContextPlanner 接入，缺失与空文件区分。

F3 固定处理点在每个采样 Step 前更新 Research/Project/Memory/Prompt 候选，随后捕获 immutable SamplingStepPlan。Domain/Phase 操作者选择仍只在 Turn 边界生效。History 在 user/model/result 检查点持久化：模型 ToolCalls 必须先落盘才执行依赖工具，真实 Receipt/Result 必须先落盘才重采样；中断未知状态不得被伪造为成功或自动重试。

Prompt renderer 与 Prompt 内容分离：当前只有固定 `generic-v1`，只拥有 fragment separator、instruction/content channel 渲染和 ContextDeliveryMap emission，不拥有人格、Domain/Phase、Tool Schema 或 Provider wire。

### 9.2 优化能力

- `/prompt inspect`：显示本次模型请求的精确组装顺序、来源、Hash 和 Token 估算。
- `/prompt edit project`：打开项目 Prompt。
- `/domain <source|web|mobile|binary>`：排队切换领域 Profile，在下一 Turn 生效。
- `/phase <mapping|hypothesis|experiment|verification|evidence_curation>`：排队切换当前研究阶段，在下一 Turn 生效。
- `/prompt diff`：对比相邻请求中的 Prompt 变化。
- `/context`：显示各片段占用和截断情况。
- 每次 Model Request 记录 Prompt Version 和 Context Fingerprint。
- 使用 snapshot test 保证组装顺序不被无意改变。
- 稳定 Prompt Fragment 独立计算 Hash 和 Token，内容未变时复用缓存。
- Base、Personal、Project、Domain 和 Phase Prompt 的变化必须导致 Context Fingerprint 变化和增量请求回退；实际发送的 Tool Exposure 变化使用 `TOOLS_CHANGED`。

### 9.3 DomainProfile 和 ResearchPhase

固定 DomainProfile 只包含：

- stable ID 和 generation；
- 编译期 Markdown Prompt；
- TargetKind 兼容性；
- bounded ToolExposurePlan。

固定 ResearchPhase 只包含：

- stable ID 和 generation；
- 当前工作目标的短 Markdown Prompt；
- bounded ToolExposurePlan。

新 Session 在第一次模型请求前追加默认 `mapping` 选择；legacy Session 的只读 Replay 只解析为 `mapping` 而不改写历史，在下一次真实 Turn 请求前再追加该默认选择。显式 operator 选择同样只有在 Session Event durable append 成功后才于 Turn 边界生效；失败则在模型请求前停止。首版不从模型自由文本或隐式启发式推断 Phase。未来只有在精确 Ledger 谓词和稳定 Reason Code 已定义时，才允许增加一个纯函数投影。

新请求使用 `prompt:domain:<id>:<generation>` 和 `prompt:phase:<id>:<generation>`。历史 `prompt:mode:<id>:<generation>` RequestAudit 保持可重建，不追溯改写。

### 9.4 漏洞研究基本原则

Base Contract 应要求 Agent：

- 保持务实、简洁、证据优先的漏洞研究协作方式；人格不随 Domain 或 Phase 变化。
- 先理解攻击面，再提出可检验假设。
- 主动使用工具验证，不把模型推理当成事实。
- 区分 Candidate、Reproduced 和 Confirmed Impact。
- 记录版本、环境、前提、正向结果和失败对照。
- 不重复已被证据否定的路径。
- 把大型证据保存为 Artifact，而不是反复填入上下文。

这些是 Prompt 约定，不是复杂的代码级审批系统。

## 10. 工具系统

### 10.1 最小接口

    trait Tool {
        fn name(&self) -> &str;
        fn spec(&self) -> ToolSpec;
        async fn execute(&self, ctx: &ToolContext, args: Value) -> ToolResult;
    }

    ToolSpec {
        name
        version
        description
        input_schema
        output_schema
        exposure: direct | deferred | hidden
        parallelism: parallel | resource_locked | serial
        resource_key_template
        effect_class: read_only | local_mutation | external_mutation | interactive
        retry_policy: replay_safe | dedupe_by_key | never_auto_retry
        determinism: deterministic | environment_dependent | nondeterministic
        cache_policy: none | content_addressed
        timeout
        output_policy
    }

每次调用在执行前解析为：

    ToolExecutionPlan {
        execution_id
        tool_name
        tool_version
        normalized_args
        resolved_resource_keys
        resolved_effect_class
        idempotency_key
        input_fingerprint
        target_snapshot_id
        environment_fingerprint
    }

执行结束产生 `ExecutionReceipt`，至少保存 Plan Hash、开始/结束状态、退出原因、输出 Artifact/Observation 引用和环境变化摘要。

模型可见的 Tool description 必须稳定包含 `Use when`、`Do not use when`、`Prefer` 和 `Produces`。已知输入/输出使用精确 JSON Schema、required/enum/bounds 和实际结构，不用无约束通用 object 掩盖契约。

执行失败统一映射为：

    ToolFailure {
        code
        stage: pre_execution | execution
        retryability: correct_arguments | explicit_rerun_only | terminal
        message
        field_errors[]?
        execution_id?
        receipt_ref?
    }

每个 ToolFailure 都必须放在与原 ToolCall 配对的 ToolResult 中；没有 ExecutionReceipt 不等于可以省略结果。只有 `pre_execution + correct_arguments` 可以在没有 ExecutionReceipt 时返回给模型纠正，因为工具尚未执行。执行一旦开始必须有真实 ExecutionReceipt，包括执行失败或取消；模型必须发起新的 ToolCall，Runtime 不静默重试。Receipt、History、RequestAudit、Artifact 等执行后持久化提交失败属于 App/AgentLoop 错误，不得把真实成功 ToolResult 改写成失败或伪造 Receipt。

Architecture v1.4 中的 `field_errors` 与 `receipt_ref` 是可选契约词汇，不构成当前 Rust 占位授权。M4-S3b 只有 `execution_id` 存在真实模型可见消费者，因此当前实现不增加无消费者字段、类型或 getter；需要它们时必须先出现真实生产路径和对应回归。

Tool Registry 只负责：

- 名称到实现的映射。
- JSON Schema 验证。
- Tool guidance、input/output Schema 和结构化错误的一致呈现。
- 超时和取消。
- 并行或串行执行。
- Direct/Deferred/Hidden 工具暴露。
- Resource Key 解析和冲突调度。
- Effect/Retry/Determinism 解析和 Execution Intent/Receipt 配对。
- 结果截断和 Artifact 存储。
- 本地审计。

### 10.2 MVP 内置工具

- `read_file`
- `read_files`
- `list_files`
- `search_files`
- `run_command`
- `run_commands`
- `write_file` / `apply_patch`
- `view_image`
- `http_request`
- `inspect_project`
- `inspect_environment`
- `artifact_store`
- `memory_read`
- `memory_write`
- `research_get`
- `research_update`
- `tool_search`

`run_command` 是漏洞工具集成的主入口，可直接调用 `rg`、`git`、`adb`、`frida`、`apktool`、`jadx`、`gdb`、`lldb`、`radare2`、`nmap`、`curl` 和用户自己的脚本。Core 不重复实现这些工具。

`http_request` 只提供结构化 method/URL/header/query/body、bounded timeout/redirect/preview，以及 status/final URL/metadata/timing/body Artifact，不演变成 Web Scanner 或厂商 API Client。

`inspect_environment` 只返回与研究相关的 OS/runtime、Git、proxy、toolchain 和 connected-device availability/version 及稳定 fingerprint，复用现有进程和 Snapshot 所有者，不建立第二个环境数据库或任意 Probe DSL。

文件修改能力由 Cyber Agent 自有 Adapter 提供，不依赖 `codex-apply-patch`：

- `write_file` 负责小文件或完整内容已知时的原子整文件写入；写入前后记录内容 Hash、大小和本地审计事件。
- `apply_patch` 作为 `ToolExecutor` 下的自有 Adapter，对上只暴露稳定 Tool Schema 和 `ToolResult`，对下可以选择精简 Patch 实现，或在 Git 项目中调用外部 `git apply --check` 后再应用。
- 首期不复制 Codex 的 Patch Parser、Grammar 和完整 Apply Patch crate；优先支持统一 Diff 的最小必要子集，复杂改动可退回 atomic whole-file write。
- Adapter 必须先验证全部 preimage，再暂存目标内容；正常校验或应用失败不能留下部分写入，进程中断后根据 Tool Event 和前后 Hash 诊断实际状态。
- 应用后返回实际 Diff、变更文件列表、前后 Hash 和失败原因；不使用会主动保留冲突碎片的 `--reject` 路径。
- Domain、Agent Loop 和 Research Ledger 只依赖通用 `ToolExecutor` Port，不感知具体 Patch 后端，后续替换实现不影响主循环。

批量工具的目标是减少模型往返：

- `read_files` 一次接收多个 path/range，分项返回结果。
- `search_files` 可一次接收多个 Pattern 和路径集。
- `run_commands` 显式声明 serial/parallel，每个子命令独立返回状态。
- 单次批量调用必须有数量、输出和总时间上限，避免一个结果占满上下文。

### 10.3 自定义工具

首期不实现插件市场和动态 Rust 插件。自定义工具使用两种方式：

1. 直接让 `run_command` 调用本地脚本。
2. 在 `.cyber-agent/tools.toml` 中声明名称、参数 Schema 和 argv 模板。

argv 必须以数组传递，不把未经处理的模型参数拼接成 shell 字符串。

MCP 只作为后续可选 Adapter；在真的有必须接入的 MCP Server 前不实现。

### 10.4 Tool Exposure 和按需发现

精简复用 Codex ToolExposure，只保留三种暴露状态：

- `Direct`：基础工具，初始请求直接暴露。
- `Deferred`：按 DomainProfile、ResearchPhase 或项目需要延迟的工具，通过 `tool_search` 发现后再追加 Schema。
- `Hidden`：由 Runtime 内部调用，模型不可直接调用。

一个纯所有者按 Registry default → DomainProfile override → ResearchPhase override → Deferred activation → runtime availability 合成最终暴露。`Hidden` 是终止状态，后续层不能搜索或提升；Phase/Domain 只能提升或延迟非 Hidden 工具。

Registry 本身是 immutable canonical catalog。每个采样 Step 的纯 resolver 生成 ordered `ResolvedToolPlan`，其 exact ToolSpecs 与 Runtime binding 一起进入 SamplingStepPlan。模型 ToolCall 只能经广告该调用的 Plan dispatch；未广告、未激活、Hidden、Unavailable 或 fingerprint mismatch 返回配对 pre-execution ToolResult，不能回查 raw catalog 或更新后的 Plan。

Deferred activation 只属于当前 Turn：成功 `tool_search` 使后续 Step 捕获新 Plan，当前 Step 不变；下一 Turn 和 Resume 清空。RequestAudit 记录每个 Step 实际发送的 ToolSpecs，因此不建立第二套 durable activation fact。

默认 Direct 只保留当前阶段高频工具以及 Research State 和 `tool_search`。Mapping 偏 inspect/read/search/batch/environment/network；Experiment/Verification 才直接暴露更多执行、Job、PTY 和变更工具；Evidence Curation 偏 Artifact/Research/Memory。它们只是模型上下文优化，Runtime 仍为 FullAccess。

当总 Tool Schema 还很小时，Deferred 不应增加额外往返。启用阈值由实测确定，至少记录 Tool Schema Token 占用和 `tool_search` 命中率。

Effect 元数据只用于调度、重试、缓存和恢复，不用于确认弹窗或安全审批：

- `read_only + deterministic + content_addressed` 的工具才允许按 Tool Version、Input Fingerprint、TargetSnapshot 和环境指纹复用结果。
- `dedupe_by_key` 只在稳定 Idempotency Key 完全一致时抑制同一执行；它不能替代 Research `action_fingerprint`。
- `external_mutation` 和 `interactive` 默认 `never_auto_retry`；模型或用户可以显式重新执行，并形成新的 execution_id 和 rerun reason。
- `run_command` 无法可靠推断副作用时使用保守的 `interactive/never_auto_retry`；在 `tools.toml` 中声明的专用脚本可以提供更精确元数据。
- Tool Result Cache 是可删除派生数据，命中时仍产生新的 Tool Event，并引用原 ExecutionReceipt/Artifact，保证审计链不断裂。

### 10.5 并行和长运行工具

- 多个只读文件工具可并行。
- 普通命令不再全局串行，而是依据 Resource Key 冲突决定。
- 常用 Resource Key：`file:<canonical-path>`、`device:<serial>`、`pty:<session>`、`job:<id>`、`output:<path>`。
- 不同文件、不同设备、不同输出的独立工具可并行。
- 无法解析 Resource Key 的外部交互命令默认串行。
- 长运行命令返回 Job ID，通过 `job poll`、`job input`、`job stop` 继续交互。
- Job Poll 只返回上次 offset 之后的新输出，不反复发送全量 stdout/stderr。
- PTY 是移动端、调试器和交互式工具的核心能力，进入基础工具里程碑，不继续延后。
- 工具开始、完成、失败和取消都形成本地事件。
- 恢复会话时不自动重放上一进程未完成的有副作用命令。
- Provider 网络重试只重试模型请求；ToolExecutionPlan 一旦获得完成 Receipt，不因模型重试再次执行。
- 执行前参数失败不产生虚假 ExecutionReceipt；执行失败和已完成执行都不因“纠错”自动重放。执行后 Receipt/History/RequestAudit 持久化失败走 App/AgentLoop 错误边界，不产生 post-execution ToolFailure。

## 11. Full Access 执行模型

默认配置：

    [execution]
    mode = "full"
    sandbox = false
    confirm_tools = false
    allow_network = true
    inherit_environment = true

语义：

- 不使用产品级权限弹窗。
- 不要求每次 Shell、网络或文件写入确认。
- 不使用 Codex 的复杂 Sandbox/Approval 矩阵。
- 工具继承启动 Cyber Agent 的本地用户权限。
- 所有操作仍写入本地审计日志，便于复盘。

Full Access 不等于无条件记录敏感值：

- 审计数据记录环境变量名，默认不记录密钥值。
- 超大 stdout/stderr 存为本地 Artifact，事件中保存摘要和 Hash。
- 不向任何 telemetry 或远程日志系统上传命令、目标、文件路径和工具结果。

## 12. 本地审计和隐私边界

### 12.1 本地事件

`events.jsonl` 使用追加写入，至少记录：

- SessionStarted / SessionResumed / SessionEnded
- UserMessage
- ModelRequest：Provider、Model、Prompt Hash、Context Fingerprint、Token Usage
- AssistantMessage / ToolCall
- ToolExecutionPlanned / ToolStarted / ToolCompleted / ToolFailed / ToolCancelled
- ExecutionReceiptRecorded
- ArtifactCreated / ArtifactDerived
- MemoryChanged / MemoryExternalChangeDetected
- PromptChanged
- ResearchPhaseSelected：Phase、generation、source 和稳定 Reason Code
- CompactionCreated / StreamRecovered

完整模型请求快照落盘保持 Deferred。F4 的 `chat --debug` 仅在开发构建输出本地 stderr，不创建 `request-snapshots/` 或第二日志；请求审计仍复用既有事实与 Blob。Provider 明文思考默认单独显示，包括 release，但不进入 Assistant stdout 或 Conversation History。

仅记录 Hash 不足以长期审计，因为 Prompt、Memory 和项目文件以后可能变化。默认审计因此保存轻量 `RequestAuditManifest`：

- 按发送顺序记录 Instructions、Input Item、Tool Schema、Output Schema、Model Profile 和语义请求属性的类型、来源、版本与内容 Hash。
- 记录 SamplingStepPlan identity、ResolvedToolPlan fingerprint、`generic-v1` renderer identity 和 ContextDeliveryMap 或其去重 Blob 引用；sent ToolSpecs 与 dispatch plan 必须可证明一致。
- 记录 ModelInputIR Schema/Hash、Provider Adapter Version 和 Capability Mapping Hash；语义 IR 与实际 Provider 编码可以分别定位。
- 每个首次出现的渲染后文本片段、Tool/Output Schema 和派生图片副本只向 Artifact Store 内的 content-addressed Blob Store 写入一次；后续请求复用同一 Blob 引用。
- User/Assistant/Tool Item 仍以 Session Event 为权威，Manifest 引用对应 Event ID；大型 Tool Output 和图片引用 Artifact ID。
- Authorization Header、API Key、连接状态、本地时间戳和无语义 Transport metadata 不进入请求内容 Blob。
- 由 Event Stream、RequestAuditManifest 和 Artifact Blob 可重建发送给 Provider 的语义请求；只有需要排查 HTTP 序列化或 Provider 差异时才打开完整原始 Request Snapshot。

这些审计 Blob 是 Event 引用的权威内容，不属于 `cache/`，不能在普通 Cache 清理中删除；使用内容寻址和引用检查控制重复与孤立数据。

MVP 不使用 SQLite、Elasticsearch 或远程日志服务。对个人项目，Markdown + TOML + JSONL + content-addressed Blob 足够，也更容易手工修复。

### 12.2 网络边界

Cyber Agent 只允许三类有意义的对外网络请求：

1. 发送到用户选择的 Model Provider 的模型请求。
2. Agent 显式调用 `http_request`、`run_command` 或漏洞研究工具产生的研究流量。
3. 用户显式触发的更新检查，且默认可关闭。

不存在隐式 telemetry、运行状态上报、远程审计、远程记忆或云同步。

如果使用云模型，被选入上下文的 Prompt、消息、图片和工具结果必然会发送给该 Model Provider。`/context` 和 `/prompt inspect` 必须让这个边界对用户可见。“不传日志”指不另行上传产品日志，不代表云模型可以在不接收输入的情况下工作。

### 12.3 本地效率指标

没有可观测数据就不能证明架构优化有效。每个 Session 在本地记录：

- Time to First Token。
- 每次 Model Request 的总耗时、Input/Output/Cache Token。
- 全量请求与增量请求次数、命中率和回退原因。
- Prompt/Context 各 Fragment 的 Token 占用和 Cache Hit/Miss。
- Tool Schema Token 和 Deferred Tool Search 命中率。
- 模型往返次数、Tool Call 次数、Batch 大小和并行度。
- 每个工具耗时、输出大小、截断量和 Artifact 大小。
- 重复文件读取比例、Project Snapshot Cache Hit 和 Memory Retrieval 命中。
- Compaction 次数、耗时和压缩前后 Token。
- 重复建立的 Hypothesis 和重复运行的已否定 Experiment。
- controlled Eval 中 acceptable first-action、invalid arguments/correction、missed batch opportunity 和 Schema cost；unmatched Replay termination 与 Observation/Replay/Vision promotion 在无 typed durable signal 前保持 Deferred，绝不假 0。Deferred activation 只可由同一 Turn 相邻 RequestAudit ToolSpec transition 证明；当前 RequestAudit 没有持久 Turn identity，因此 M4-S3b 保持 Deferred。
- Retrieval Shadow 的 recall/precision、重复读取/Token 变化、authority/freshness disposition，以及 Guidance Shadow 的关键 gap closure/cost。

指标只写当前项目的本地 Event/summary，不上传。`/metrics` 显示当前 Session 和项目累计值。

默认审计级别不保存每次完整 Request Snapshot，只保存事件、Hash、Fragment Manifest 和指标。完整 Snapshot 文件持久化保持 Deferred；F4 的开发构建 `chat --debug` 只输出脱敏后的请求/响应到 stderr，不新增文件或重复事实源。

### 12.4 Trace Replay 和 Shadow Eval

Event、RequestAuditManifest、ExecutionReceipt 和 Artifact 已足以构造可重放 Trace，不再建立第二套 Trace 事实源。

    TraceFixtureManifest {
        trace_id
        source_session_id
        event_range
        source_stream_hash
        target_snapshot_ids
        required_artifact_ids
        provider_event_refs
        tool_receipt_refs
        expected_invariants
    }

提供三种严格区分的模式：

1. Mechanical Replay
   - 使用 `ReplayModelProvider` 和 `ReplayToolExecutor` 返回历史 ModelEvent、ToolResult 和 Receipt。
   - 不发送模型请求，不执行真实命令，用于验证 AgentLoop、Reducer、Tool Pairing、Compaction 和恢复。

2. Context Recompile
   - 对历史 Turn 使用新的 Prompt/Context Planner 重新生成 ContextDecisionManifest。
   - 只比较 Token、片段选择、失效原因和 RequestAuditManifest，不把历史模型输出当作新 Prompt 的质量结论。

3. Shadow Model Eval
   - 仅在用户显式触发时向所选 Provider 发送新的 ModelInputIR，但 ToolExecutor 固定为 Replay。
   - 新模型请求了 Trace 中不存在的工具调用时立即标记 `UNMATCHED_TOOL_CALL`，默认不回退真实工具执行。

约束：

- Replay/Shadow 使用独立 Eval Session 或 namespace，默认不写 Project Memory 和 Research Ledger。
- Replayed/Shadow 结果必须带来源标签，不能被提升为真实漏洞证据、复现或 confirmed Finding。
- Trace Fixture 只保存 Event/Artifact 引用和期望不变量；源事实仍是原 Event Stream 和 Blob。
- Prompt、Context、Provider Adapter 或 Tool Protocol 修改后，优先运行固定 Trace Suite，再进行真实目标研究。
- Tool Accuracy Eval 只从现有 Trace/Event/ToolCall/Result/Receipt/Artifact 派生报告；可为一个 case 声明多个 acceptable first actions，不建立模型裁判服务。
- 受控报告至少比较首次动作、参数失败/纠正、未匹配 Tool、重复读取、遗漏 Batch、Deferred Search、Schema Token 和证据提升边界；报告不能写正式 Research/Memory。

### 12.5 确定性安全检索与证据引导

安全检索不是通用 Skill selector。它使用 7.4 的固定 pipeline，并针对 CVE/CWE、package/bundle、class/method/symbol、URL、Android permission/intent/component、binary import/export/address、Hash 和 Research/Artifact/Receipt ID 保留标点语义。不同 candidate pool 只能复用 normalizer/ranker，不能共用一个 global eligibility 或 opaque score。

检索先在 local Shadow 运行：不改变 ModelInputIR，不调用 Provider/Tool，不写 Research/Memory。报告包含 algorithm/query identity、rank components、authority/freshness tier 和成本；没有 measurable benefit 时保持 Shadow 或删除。

一个独立 pure evidence-gap projection 只从 typed Research fields/EvidenceLinks 推导 entry/sink/reachability/prerequisite/positive/negative/environment control/impact/repeatability/target-version/provenance/contradiction gaps。它不把 Unknown 当 Missing，也不改变 Phase 或 Finding。

实验引导只排序既有 NextAction 或 caller-supplied Eval candidates，factor 固定为 boundary importance、competing-hypothesis discrimination、expected gap closure、reproducibility、novelty、Target applicability、execution cost、mutation risk 和 version-drift risk。Hard invalidity/missing prerequisite 先过滤，所有 factor 和 tie-break 可解释。

Guidance 从不创建 Experiment、执行 Tool、更新 Memory/Research 或自动切换 ResearchPhase。它先运行 Shadow；不建立 learned ranker、model judge、autonomous workflow 或 experiment-template DSL。

## 13. 多模态

MVP 保留图片，不实现图片生成和语音交互。

图片流程：

    local image
      ↓
    validate format and size
      ↓
    hash and store local artifact
      ↓
    resize only when context budget requires it
      ↓
    resolve VisionMode against explicit ModelCapabilities
      ├── primary_direct → selected image enters primary ModelInputIR
      ├── secondary_tool → scoped VisionAnalysisService → VisionObservation
      └── disabled/no capable provider → typed capability error
      ↓
    keep original path, hash, route decision and derived metadata locally

支持：

- 用户在 CLI 中附加截图。
- `view_image` 工具返回图片。
- 崩溃界面、设备界面、反编译器截图和工具生成图片。
- Provider 不支持图片时给出明确错误，不静默丢弃。
- 单一多模态主模型可以直接处理全部文本、图片和工具请求，不要求配置第二模型。
- 纯文本主模型可以配置一个视觉模型；视觉模型输出属于派生 Observation，不自动成为已验证事实。
- 初次用户图片在 `secondary_tool` 下必须先得到显式分析，不能仅把不可见的 Artifact 占位符交给主模型后继续。

## 14. Model Provider

Core 使用 Provider-neutral 的语义请求，不直接暴露 OpenAI Responses HTTP 字段：

    ModelInputIR {
        instructions
        content_items
        tool_specs
        output_contract
        reasoning_options
        semantic_request_properties
        delivery: full | append_only
        logical_input_manifest
        context_decision_manifest
    }

`delivery=append_only` 只表示语义输入相对 Baseline 可追加；具体 `previous_response_id`、连接复用和 Provider Request ID 由 Adapter/ProviderSession 管理，不进入通用 Domain 协议。

Provider 接口保持最小：

    trait ModelProvider {
        async fn stream(&self, input: ModelInputIR) -> ModelEventStream;
        fn capabilities(&self) -> ModelCapabilities;
    }

- `ResponsesHttpAdapter` 将 ModelInputIR 编码为 Responses HTTP Streaming 请求，并把 Provider Event 解码回通用 ModelEvent。
- Responses request/response 类型只存在于协议 Adapter 内，不能泄漏到 `domain`、Context Planner、Research Ledger 或 Tool Runtime。
- Adapter 必须显式校验文本、图片、Tool、Output Contract、Usage 和增量能力；不支持的 ContentItem 直接返回能力错误，不静默丢弃或隐式降级。
- 后续符合已实现 Responses 契约的服务只增加配置和 capability mapping，不增加厂商 Adapter，不建设模型路由 Server，也不修改 AgentLoop 语义。
- Capability Mapping 是带版本的显式配置；不能根据 Model slug、Provider 品牌或字符串前缀猜测图片、Tool、Streaming 和增量能力。

AgentLoop 只拥有一个 Primary Provider；可选 Vision Provider 通过同一个 `ModelProvider` Port 被 `VisionAnalysisService` 受限调用：

    AgentModelProfile {
        primary_profile_id
        vision_profile_id?
        vision_mode: auto | primary_direct | secondary_tool | disabled
    }

- `auto`：主模型声明支持图片时直传；否则使用已配置且声明支持图片的视觉模型；两者均不可用时明确失败。
- `primary_direct`：所有选中图片进入主模型，主模型未声明图片能力时配置无效。
- `secondary_tool`：图片由视觉模型生成结构化 VisionObservation，再进入主模型上下文；没有视觉模型时配置无效。
- `disabled`：文本功能保持可用，任何图片输入返回明确能力错误。
- `auto` 不按价格、延迟、临时故障或输出质量切换模型；Provider 失败也不静默 fallback。

进程内、本地 Session 级 `ProviderSession` 另外保存；`TurnContext` 只捕获该 Provider Session 在当前 Turn 的稳定引用和配置代次：

- 当前 Model Profile。
- 稳定 Prompt Cache Key。
- Last Full Request Fingerprint。
- Last Response ID。
- 服务端已返回的 Output Items。
- Transport 状态和 retry count。
- Incremental Baseline 和最后一次回退原因。

MVP：

- 一个协议优先的 Responses HTTP/SSE Provider Adapter，可由配置指向任意符合已实现协议契约的服务。
- 配置 Base URL、显式 API Key 来源（内联值或 Cyber Agent 专用环境变量引用）、Model 和 Context Window。
- Adapter 支持文本、图片、Function Tool Call 和 Usage，但实际 Profile 必须以显式 Capability Mapping 为准；Primary 不强制具备图片能力。
- 同一运行 Session 内连续工具调用和跨 Turn 追加用户 Input。
- `previous_response_id + incremental input` 增量请求。
- 非 Input 属性一致性比较和严格 Input 追加比较。
- 增量条件不成立时透明回退完整 HTTP Streaming 请求，并记录回退原因。
- 网络重试不重复执行本地工具。
- 一个主模型加至多一个可选视觉模型；支持纯文本主模型 + 视觉模型，以及单一多模态主模型两种部署。

Provider 凭据边界属于 Adapter/Composition Root 配置，不进入 Domain：

- Provider Config 为每个 Provider 显式指定且只指定一个内联 `api_key` 或 `CYBER_AGENT_*_API_KEY` `credential_ref`；两种形式都只提供 Secret，不标识或推断 Provider/Model，也不回退读取通用厂商变量。
- 不读取或修改 `~/.codex`、`CODEX_HOME`、Codex OAuth/session data，不调用 Codex CLI 获取凭据。
- Mock、Fixture 和 Replay 路径无凭据、无公网即可运行；真实 Provider 测试默认 ignored，必须由 `CYBER_AGENT_LIVE_TESTS=1` 显式开启。
- 本地个人配置允许直接保存 Key；GitHub Actions 仍使用可单独撤销的引用式测试 Key。两者都不共享当前 Codex 登录，也不把 Key 写入 Event、Artifact、Snapshot、日志或被提交的仓库配置。

WebSocket 和 HTTP/SSE 连接复用在 HTTP Streaming 稳定后评估；这是 Transport 优化，不阻塞 MVP 的 `previous_response_id` 增量语义。

后续如果有真实需求，再增加 OpenAI-compatible 或 Local Model Provider。不建立 Provider 插件平台。

### 14.0 Vision Provider 隔离

- Primary ProviderSession 是 AgentLoop 唯一可增量复用的 ProviderSession。
- Secondary Vision Request 使用独立的 scoped request identity，不与 Primary 共享 `previous_response_id`、Server Output Items 或 IncrementalBaseline。
- `VisionRouteDecision` 记录 VisionMode、Resolved Route、Reason Code、AgentModelProfile 引用、Capability Hash 和 source Artifact。
- `VisionObservation` 记录 source/derived Artifact、task hash、摘要、OCR/region、线索、不确定性和 RequestAuditManifest 引用。
- Replay 只重放已记录的 Route/Observation，不调用真实 Vision Provider。
- Vision Provider 没有项目 Memory、Research Ledger、通用 Tool Runtime 或独立自动续写能力，因此不构成第二 Agent。

### 14.1 Incremental Baseline 和失效矩阵

    IncrementalBaseline {
        provider_session_id
        model_input_ir_schema_hash
        provider_capability_hash
        model_profile_hash
        instructions_hash
        tool_schema_hash
        output_schema_hash
        request_properties_hash
        canonical_input_items
        server_output_items
        last_response_id
    }

请求属性在计算 Hash 前必须使用稳定字段顺序和稳定 Tool 顺序序列化。Internal ID、本地时间戳和不影响请求语义的 metadata 不参与内容等价比较。

| 情况 | 处理 |
|---|---|
| Model/Profile 变化 | 清空 Baseline，发送全量请求 |
| ModelInputIR Schema 或 Provider Capability Mapping 变化 | 清空 Baseline，发送全量请求 |
| Base/Personal/Project/Domain/ResearchPhase Prompt Hash 变化 | 清空 Baseline，发送全量请求 |
| Tool Schema/Exposure 变化，包括 Deferred Tool 加载 | 清空 Baseline，发送全量请求 |
| Output Schema、reasoning、include 或其他非 Input 语义变化 | 清空 Baseline，发送全量请求 |
| Input 严格等于旧 Input + 服务端 Output + 新增 Items | 发送 `previous_response_id + delta` |
| History 中间项被修改、删除或重排 | 清空 Baseline，发送全量请求 |
| Compaction/Replacement History | 清空 Baseline，下一次真实请求发送全量 Context |
| 进程重启或 Session Resume | 不复用旧 Response ID，首次请求全量发送 |
| Provider 拒绝、过期或找不到 Previous Response | 清空 Baseline，同一模型请求最多回退全量重试一次 |
| 同一 Transport Request 在未获取有效 Response 前失败 | 可重试该模型请求，不重放任何本地 Tool Call |

补充规则：

- `/profile` 切换在当前 Turn 结束后生效，并创建新 Provider Session。
- Last Response ID 可写审计事件，但默认不作为跨进程恢复优化。
- 每次回退记录结构化 Reason Code，例如 `MODEL_CHANGED`、`TOOLS_CHANGED`、`INPUT_NOT_APPEND_ONLY`、`COMPACTION_RESET`、`PROVIDER_REJECTED`。
- RequestAuditManifest 同时记录增量判断使用的 ModelInputIR Hash 和 Adapter/Capability 版本，便于重放时区分 Planner 变化与 Provider 编码变化。
- 增量比较和回退必须有独立单元测试和请求 Snapshot Test。

### 14.2 Model Profile

不实现自动模型网关，只保留可命名的 Provider Profile 和组合后的 Agent Profile：

    [providers.deepseek_fast]
    model = "..."
    reasoning_effort = "low"

    [providers.multimodal_primary]
    model = "..."
    reasoning_effort = "high"

    [agent_profiles.default]
    primary = "deepseek_fast"
    vision = "multimodal_primary"
    vision_mode = "auto"

- Provider Profile 定义 Adapter、Model、Capability Mapping、显式凭据来源和请求属性。
- Agent Profile 只组合一个 Primary、可选 Vision 和 VisionMode，不包含通用路由规则。
- 用户可用 `/profile <agent-profile-name>` 切换组合 Profile。
- DomainProfile/ResearchPhase 不选择 Provider；AgentModelProfile 只由显式配置/操作切换，不在单个 Turn 中隐式频繁切换模型。
- `/profile` 或 `/vision mode` 在当前 Turn 结束后生效；改变 Primary 请求语义时重建 ProviderSession 和 Baseline。
- `doctor` 检查 AgentModelProfile、VisionMode 与显式 Capability 是否匹配；它会解析内联配置但不显示 Secret，也不发起 Provider 请求。

Provider Config v1 使用显式 `schema_version`、`active_agent_profile`、命名 `providers`、命名 `capability_mappings` 和命名 `agent_profiles`。Provider 条目分别声明固定 binding、Wire API、Base URL、Model、二选一的内联 `api_key` 或专用 `CYBER_AGENT_*_API_KEY` 引用、Capability Mapping 引用和 generation；未知字段、凭据缺失/冲突、悬空引用、零版本和非法能力组合必须在网络活动前失败。

`chat`、`doctor` 和通用 Live Smoke 只能经过同一个 Provider Composition Root。通用路径不包含 Provider 官方 URL、模型环境变量或直接 Adapter 构造。协议专属离线 wire/contract test 可以直接构造其 Adapter。

协议实现按 Wire API 复用：`binding = "http_sse"` 与 `wire_api = "responses"` 只有一个 request encoding、HTTP/SSE transport 和 response decoding 所有者。URL、Model、凭据来源和 Capability Mapping 只产生配置实例，不产生厂商 binding；只有 fixture 证明的真实 wire 差异才允许扩展协议实现。不建设动态 Registry、通用 Router、自动 fallback 或插件系统。具体决策见 ADR-0006、ADR-0007 与 ADR-0008。

## 15. 交互界面

### 15.1 CLI 优先

MVP 使用可交互 CLI，提供流式输出、命令历史、多行输入、Ctrl-C 取消和会话恢复。

启动命令：

    cyber-agent init
    cyber-agent chat
    cyber-agent chat --resume
    cyber-agent chat --project <path>
    cyber-agent inspect audit
    cyber-agent inspect context
    cyber-agent inspect research
    cyber-agent inspect snapshot
    cyber-agent inspect metrics
    cyber-agent memory edit

REPL 内部命令：

- `/project`
- `/new`
- `/resume`
- `/memory`
- `/research`
- `/hypotheses`
- `/experiments`
- `/findings`
- `/remember`
- `/forget`
- `/prompt`
- `/context`
- `/compact`
- `/tools`
- `/tools search <query>`
- `/jobs`
- `/domain <source|web|mobile|binary>`
- `/phase <mapping|hypothesis|experiment|verification|evidence_curation>`
- `/profile <name>`
- `/vision mode <auto|primary_direct|secondary_tool|disabled>`
- `/snapshot`
- `/metrics`
- `/image <path>`
- `/audit`
- `/exit`

工具调用默认展示工具名、关键参数、持续时间和退出状态，但不要求用户批准。

### 15.2 可选本地 Web

Web 不进入 MVP。CLI 稳定后，如果图片、Markdown 和长输出的展示仍不方便，再增加：

    cyber-agent web --bind 127.0.0.1

要求：

- 只是本地 UI，复用同一 Agent Runtime 和项目数据。
- 默认只绑定 loopback。
- 不提供多用户、登录、远程部署和云端会话。
- 不实现账号、鉴权和远程访问；非 loopback 暴露不在项目范围内。
- 作为 Cargo feature 可选编译，不让 Web 依赖进入默认 CLI 二进制文件。

## 16. 代码结构

首期只使用一个 Cargo package：

    cyber-agent/
    ├── Cargo.toml
    ├── task.md
    ├── prompts/
    │   ├── base.md
    │   ├── domains/
    │   └── phases/
    ├── src/
    │   ├── main.rs
    │   ├── lib.rs
    │   ├── domain/
    │   │   ├── events.rs
    │   │   ├── model.rs
    │   │   ├── research.rs
    │   │   ├── context.rs
    │   │   ├── domain_profile.rs
    │   │   ├── research_phase.rs
    │   │   ├── tools.rs
    │   │   └── artifacts.rs
    │   ├── ports/
    │   │   ├── model_provider.rs
    │   │   ├── tool_executor.rs
    │   │   ├── event_store.rs
    │   │   ├── artifact_store.rs
    │   │   └── cache_store.rs
    │   ├── app/
    │   │   ├── agent_loop.rs
    │   │   ├── session.rs
    │   │   ├── provider_session.rs
    │   │   ├── context_planner.rs
    │   │   ├── project_manager.rs
    │   │   ├── memory_manager.rs
    │   │   ├── research_service.rs
    │   │   ├── vision_analysis_service.rs
    │   │   └── eval_service.rs
    │   ├── adapters/
    │   │   ├── responses/                     # 唯一 Responses HTTP/SSE wire owner
    │   │   ├── model/replay.rs
    │   │   ├── tools/filesystem.rs
    │   │   ├── tools/process.rs
    │   │   ├── tools/pty.rs
    │   │   ├── tools/image.rs
    │   │   ├── tools/replay.rs
    │   │   ├── store/jsonl.rs
    │   │   ├── store/markdown.rs
    │   │   └── store/artifacts.rs
    │   ├── ui/cli.rs
    │   ├── provider_composition/          # 唯一 Config v1 校验与 Provider 装配入口
    │   └── support/
    │       ├── hashing.rs
    │       └── metrics.rs
    └── tests/

约束：

- 不创建 Cargo workspace。
- 不拆 `protocol`、`core`、`tools`、`memory` 等多个 crate。
- `lib.rs` 提供测试所需的内部 API，`main.rs` 只负责参数解析和启动。
- 模块间通过精简 Rust type/trait 协作，不为内部调用建立网络 API。
- 只有当独立特性、依赖隔离或外部 SDK 需求被真实证明时才拆 crate。

### 16.1 单 crate 内部依赖方向

箭头表示可以依赖：

    ui/main ───→ app ───→ ports ───→ domain
       │                     ↑          ↑
       └───────→ adapters ──────────┘

    domain / app / adapters ───→ support

规则：

- `domain` 只包含数据类型、纯 Reducer、纯去重逻辑和序列化协议，不执行文件、网络、进程或时钟 I/O。
- `ports` 定义 Model、Tool、Event Store、Artifact Store 和 Cache Store trait，只依赖 `domain`。
- `app` 实现 AgentLoop 和用例编排，只通过 `ports` 访问外部能力，不直接调用 `reqwest`、`tokio::process`、文件 API 或 JSONL 实现。
- `adapters` 实现 `ports`，可依赖 `ports`、`domain` 和第三方库，不反向依赖 `app`。
- `ui/main` 是 Composition Root，创建 Adapter，将 trait object/泛型实现注入 `app`。
- `support` 只保存无业务所有权的小型通用实现，禁止演变为新的杂物层。
- 代码审查中禁止 `domain → app/adapters/ui`、`ports → app/adapters/ui` 和 `adapters → app` 依赖。

## 17. 核心数据类型

P0/MVP 核心只需要：

- `Project`
- `ProjectFingerprint`
- `ProjectSnapshot`
- `Session`
- `EventEnvelope<T>`
- `SessionEvent`
- `TurnContext`
- `DomainProfile`
- `ResearchPhase`
- `ResearchPhaseSelection`
- `TurnConfigurationSnapshot`
- `StepSnapshot`
- `SamplingStepPlan`
- `Message`
- `ContentItem`
- `ContextFragment`
- `ContextPlan`
- `ContextDecisionManifest`
- `ContextSlot`
- `ContextOrigin`
- `ContextSubitemId`
- `ContextDeliveryMap`
- `PromptRendererIdentity`
- `ModelInputIR`
- `ModelEvent`
- `ModelCapabilities`
- `AgentModelProfile`
- `VisionMode`
- `VisionRouteDecision`
- `VisionObservation`
- `ProviderSessionState`
- `IncrementalBaseline`
- `ModelProfile`
- `ToolSpec`
- `RegisteredTool`
- `ResolvedToolPlan`
- `DeferredActivationSet`
- `ToolExecutionPlan`
- `ExecutionReceipt`
- `ToolCall`
- `ToolResult`
- `Job`
- `MemoryUpdate`
- `ResearchEvent`
- `ResearchState`
- `TargetSnapshotRecord`
- `ExperimentCapsuleRecord`
- `HypothesisRecord`
- `ExperimentRecord`
- `FindingRecord`
- `NextActionRecord`
- `BlobId`
- `ArtifactRecord`
- `AuditEvent`
- `RequestAuditManifest`
- `PerformanceSample`

P1/M4 Eval 和领域工具协议再增加：

- `ObservationEnvelope`
- `TargetDelta`
- `TraceFixtureManifest`
- `ReplayResult`
- `EvidenceLink`
- `EvidenceSpine`
- `NarrativeSummary`
- `SecurityRetrievalReport`
- `EvidenceGapReport`
- `ExperimentGuidanceReport`

这些类型只表达核心数据和效率语义，不包含团队、权限、云任务、通用工作流和复杂状态验证。Report 继续先使用 Markdown 模板，不进入 MVP 协议。

## 18. 从 Codex 复用什么

复用原则：优先 REFERENCE 后小型 REIMPLEMENT；只有 Port Ledger 先获准时才移植独立算法和测试，不依赖完整 `codex-core`。当前没有 Codex PORT。

| 能力 | 方式 | 决策 |
|---|---|---|
| Tool Call / Tool Result 配对 | REIMPLEMENT | 已按本地 History 类型重实现严格配对；上游只作边界参考 |
| ContextualUserFragment | REIMPLEMENT | 精简为 ContextFragment，保留来源、顺序、Hash 和大小上限 |
| TurnContext / StepContext | REIMPLEMENT | 保留 Turn 稳定状态与每次采样快照的分离 |
| SamplingStepPlan / ToolRouter 边界 | REIMPLEMENT | 借鉴 captured Step/Router 一致性；本地只保留 immutable plan、无 Code Mode/MCP/Policy |
| Tool Schema 与 Executor 绑定 | REIMPLEMENT | immutable ResolvedToolPlan 同时绑定 advertised spec 与 runtime，不复制上游 Router |
| ToolExposure / Tool Search | REIMPLEMENT | current-Turn DeferredActivationSet，不依赖 `codex-tools` |
| 输出截断 | REIMPLEMENT | 按本地 Tool/Context budget 类型实现，上游只作测试边界参考 |
| 图片验证和缩放 | REIMPLEMENT | 已按本地 Artifact/预算边界实现最小读取、验证、缩放和模型输入转换 |
| Streaming Responses | REIMPLEMENT | 协议优先 Responses HTTP/SSE owner 已独立实现 |
| Provider-neutral ModelInputIR | REIMPLEMENT | Domain 只表达语义请求，OpenAI 字段留在 Adapter |
| Provider Session | REIMPLEMENT | 保留同进程 Session 内的 Last Request、Last Response ID 和 Transport 状态；不跨进程复用 |
| previous_response_id | REIMPLEMENT | ProviderSession/Baseline 已按本地 IR 和 Capability 语义实现 |
| WebSocket 连接复用 | LATER | HTTP Streaming 和增量语义稳定后再评估 |
| Prompt Cache 稳定 ID | REIMPLEMENT | 复用稳定 Hash 思路，不复制实现 |
| Prompt renderer identity | REIMPLEMENT | 只有固定 `generic-v1`，不移植 model-catalog template/registry |
| 稳定人格 / DomainProfile / ResearchPhase | REIMPLEMENT | 借鉴人格只控制协作风格的分层思路；本地使用一个 Base Contract 和固定正交轴，不移植角色系统 |
| Context provenance / Provider item ID | REIMPLEMENT | Domain 使用 Fragment/Subitem + ContextDeliveryMap；DeliveryItemId 留在 Adapter |
| Replacement History / Compaction | REIMPLEMENT | Evidence Spine 只引用 Research/Artifact/Receipt facts，Narrative 保持有损非权威 |
| BM25 / char n-gram / RRF Shadow selectors | REFERENCE/REIMPLEMENT | 借鉴 deterministic primitives/Shadow gate；按漏洞标识符、Target 和 evidence authority 重实现固定 pipeline，不移植 selector catalog/telemetry |
| Apply Patch | REIMPLEMENT MINIMAL | 自有 Tool Adapter；首期只用 atomic whole-file write、最小统一 Diff 或外部 `git apply`，不依赖 `codex-apply-patch` |
| Shell 命令处理 | REIMPLEMENT | 只保留 argv、取消、截断和 Job 语义；上游作边界参考 |
| PTY | REIMPLEMENT | 直接使用 `portable-pty` 实现当前平台需要的最小功能，不移植 Codex 完整 PTY Runtime |
| World State snapshot/diff | REIMPLEMENT | 只保留轻量 ResearchWorldState Section、Snapshot、Hash 和 Diff |
| Artifact Lineage / Experiment Capsule | REIMPLEMENT | 基于现有 Event/Artifact 引用，不引入 Evidence 数据库 |
| Trace Replay / Shadow Eval | REIMPLEMENT | Replay Adapter 只消费本地 Trace，不移植 Codex 产品级 Eval Runtime |
| Observation / Target Delta | REIMPLEMENT | 使用漏洞研究字段和外部工具输出，不建立通用知识图谱 |
| Hook Runtime | DROP | 主循环内保留固定前后处理点 |
| Policy / Sandbox / Approval | DROP | 默认 FullAccess |
| TUI / App Server / Cloud / Connectors | DROP | 不符合个人工具定位 |
| Multi-agent | DROP | 不实现 |
| Research Ledger | REIMPLEMENT | 使用小型 JSONL 事件和稳定 ID，不移植完整 Evidence/Validator 产品层 |

### 18.1 禁止的直接依赖

Cyber Agent 首期 Cargo.toml 不直接依赖：

- `codex-core`
- `codex-tools`
- `codex-apply-patch`
- `codex-utils-pty`
- `app-server*`
- `codex-connectors`
- `codex-code-mode`
- `codex-cloud-*`

原因：这些 crate 会通过 workspace dependency 带入大量产品层、跨平台或服务依赖，破坏单 package 的维护目标。

### 18.2 PORT 预算

一项 Codex 代码只有在满足以下条件时才进入 PORT：

- 能明确列出少量原始文件和对应测试。
- 可以替换 Codex 专有类型，不引入完整 `codex-protocol`、`codex-core` 或 workspace dependency 闭包。
- 移植后代码能在 Cyber Agent 单 package 中独立测试。
- 本地需求只占上游功能的一小部分时，优先重实现而不是复制整个模块。
- 引入后必须记录代码行数、新增依赖、编译时间变化和上游同步成本。

默认使用 `REFERENCE -> REIMPLEMENT -> PORT SUBSET` 的升级顺序。预计超过约 200 行生产代码、涉及超过两个上游实现文件、引入新的依赖闭包、没有唯一的本地所有者或无法说明删除条件时，必须先写 ADR 或退回 REIMPLEMENT。

任何 PORT 代码必须记录：

- 上游提交。
- 原始文件路径。
- 本地改动。
- License 和 NOTICE。

## 19. MVP

MVP 必须完成：

1. 在两个不同目录创建两个独立项目。
2. 启动交互 CLI 并流式输出模型回复。
3. 默认 FullAccess 调用 Shell、文件和网络工具。
4. 模型可以连续完成 Tool Call → Tool Result → 再采样。
5. Provider Session 可判断并执行 `previous_response_id` 增量请求，不兼容时正确回退。
6. `read_files`、`search_files` 和 `run_commands` 可减少模型往返，工具按 Resource Key 并行或串行。
7. PTY 和长运行 Job 可增量读取输出。
8. EventEnvelope + 纯 Reducer 可重放 Session/Research Stream，Research Ledger 可持久化 Hypothesis、Experiment、Finding Record 和 Next Action。
9. Project Snapshot 可识别项目变化，未变化时复用缓存。
10. Project Memory Core、按需检索的 Notes、Session Summary 和失败路径可持久保存。
11. ResearchWorldState 和 Project Snapshot 支持 Full/Diff，Context Fragment 支持 Hash/Cache。
12. 会话超过上下文预算后可压缩并继续，强制退出后可 resume。
13. 可用 `/prompt inspect`、`/context`、`/research`、`/snapshot` 和 `/metrics` 解释当前状态和开销。
14. Direct/Deferred/Hidden Tool Exposure 可根据 DomainProfile、ResearchPhase 和 Schema Token 开销运行。
15. 文本和本地图片可进入模型；既支持单一多模态 Primary，也支持文本 Primary + 可选 Vision Provider，并且路由可解释、可重放。
16. 工具、记忆、Research、Prompt、Compaction、缓存命中和模型请求在本地可追溯。
17. 除用户选择的 Model Provider 和显式研究请求外，不发送任何远程日志。
18. Core 使用 Provider-neutral ModelInputIR，OpenAI Responses 字段只存在于 Adapter。
19. Research 使用不可变 TargetSnapshot 和 ExperimentCapsule 固定版本、环境、输入和执行回执。
20. ToolExecutionPlan/ExecutionReceipt 明确 Effect、Retry、Idempotency 和可缓存条件，但不增加审批。
21. Artifact 内容和语义身份分离，可从 Event 生成完整 Lineage。
22. 所有已发布 Event Schema 保留连续 Upcaster 链，旧项目不会因多次升级而失去可读性。
23. 一个稳定漏洞研究人格与正交 DomainProfile/ResearchPhase 在 Turn 边界组合，Resume 后可恢复 Phase。
24. Tool guidance、输入/输出 Schema 和结构化失败可审计；参数纠错不伪造 Receipt，执行不自动重放；执行后 Receipt/History/RequestAudit 持久化失败属于 App/AgentLoop 错误。
25. 受控 Replay/Shadow Eval 只报告持久 Trace 可诚实观测的首次工具选择、参数失败、重复读取、Batch 和 Deferred Search；unmatched Replay termination 与 Observation/Replay/Vision promotion 在无 typed durable signal 前保持 Deferred，绝不假 0。
26. 每个 Sampling Step 的 Context、ToolSpecs、RequestAudit 和 dispatch 绑定同一 immutable Plan；Deferred activation 不跨 Turn/Resume。
27. `generic-v1` renderer 和 ContextDeliveryMap 可重建精确 bytes 与 Fragment/Subitem 来源，Provider item ID 不成为 Domain identity。
28. EvidenceLink 显式表达支持/反证/控制/证明角色；Summary v2 Evidence Spine 只引用权威事实。
29. 安全检索在 Shadow 中证明 identifier/CJK/authority/freshness 排序收益后才进入 Context，派生 index 可删除。
30. Evidence-gap/NextAction guidance 可解释且不自动改变 Phase、执行 Tool、创建 Experiment 或提升 Finding。

MVP 不包含：

- Web UI。
- MCP。
- SQLite。
- 多 Agent。
- Sandbox 和审批系统。
- 强制 Finding 状态机、Validator 和审批流程。
- 多人格、角色 Agent、Domain × Phase 组合角色和通用 Workflow/Prompt Registry。
- 报告平台。
- 云服务。

### 19.1 超前设计分层

- P0 Contract：ModelInputIR、TargetSnapshot、ExperimentCapsule、ArtifactRecord、Tool Effect/Receipt、Event Upcaster、VisionMode/RouteDecision、VisionObservation 以及 ResearchPhase Session continuity 进入 MVP，因为晚加会改动核心协议和历史数据。
- P1 Validation：Trace Replay、Context Recompile、Tool Accuracy Report、ObservationEnvelope 和 TargetDelta 进入 M3/M4；先用本地 Trace 和可控样例证明价值。
- P2 Deferred：通用知识图谱、向量数据库、模型路由服务、分布式执行、动态插件 ABI 和 Workflow DSL 不预留实现，只在真实瓶颈出现后重新评估。
- P1/P2 不得创建新的权威数据库；优先从现有 Event、Research 和 Artifact 派生。

## 20. 实施里程碑

以下 checkbox 是目标能力清单，不是当前完成状态；历史交付、当前 Slice 和授权状态只以 `docs/development/STATUS.md` 为准。

### M0：单 package 骨架

- [x] 创建 `Cargo.toml`、`src/lib.rs` 和 `src/main.rs`。
- [x] 创建 `init`、`chat` 和 `doctor` 命令空壳。
- [ ] 定义最小配置和 Project 发现（按可执行计划在 M2-S1 落地，不阻塞无配置的 M1a Mock 路径）。
- [x] 建立 `domain ← ports ← app` 和 `domain/ports ← adapters` 的内部模块边界。
- [x] 定义带 Schema Family/Version 的 EventEnvelope、Upcaster 契约和纯 Reducer 最小类型。
- [x] 固定工具链 GitHub Actions 的 `cargo fmt`、`cargo clippy`、`cargo test` 和 `cargo build --locked` 通过。

完成标准：单一二进制文件可运行，没有 workspace 和额外 crate。

### M1：高吞吐最小 Agent 循环

M1 分为三个必须独立编译、独立测试、独立交付的 Vertical Slice。

#### M1a：全量请求和单工具闭环

- [x] ModelProvider port。
- [x] ToolExecutor port。
- [x] Mock Provider。
- [x] OpenAI Responses HTTP Streaming 全量请求。
- [x] Provider-neutral ModelInputIR 和 OpenAIResponsesAdapter 编解码边界。
- [x] Message / ToolCall / ToolResult。
- [x] ToolSpec Effect 元数据、ToolExecutionPlan 和 ExecutionReceipt 最小类型。
- [x] 内存 History 和 Tool Call/Result 配对。
- [x] 最小 Tool Registry。
- [x] `read_file`、`list_files`、`search_files`、`run_command`。
- [x] Tool Call → Tool Result → 全量再采样集成测试。

M1a 完成标准：在 FullAccess 下完成一次真实文件分析和工具回传；无 Provider Session、批量、PTY 也可单独运行。

#### M1b：Provider Session 和增量请求

- [ ] TurnContext、StepSnapshot 和 ProviderSession。
- [ ] IncrementalBaseline。
- [ ] 稳定请求序列化和 Prompt Cache Key。
- [ ] 非 Input 属性比较和严格 append-only Input 比较。
- [ ] `previous_response_id + delta`。
- [ ] 失效矩阵、Reason Code 和全量回退。
- [ ] 同进程、同 Session 跨 Turn 的 append-only 增量复用测试。
- [ ] Compaction/Resume/Profile/Tool Schema/ModelInputIR Schema/Capability Mapping 变化的 Baseline Reset 测试。

M1b 完成标准：兼容的第二次采样或同进程下一 Turn 只发送增量 Input；每个不兼容条件都有稳定回退原因和快照测试，进程重启或 Resume 不复用旧 Response ID。

#### M1c：批量、Job 和 Resource Key

- [ ] `read_files`、多 Pattern `search_files` 和 `run_commands`。
- [ ] Resource Key 解析、冲突锁和无冲突并行。
- [ ] Retry Policy、Idempotency Key 和完成 Receipt 防重复执行。
- [ ] 普通后台进程、Job ID 和增量 stdout/stderr offset。
- [ ] 在普通 Job 稳定后接入 PTY。
- [ ] 基础 PerformanceSample。
- [ ] 批量上限、部分失败和资源冲突测试。

M1c 完成标准：一次批量工具可读取多个文件；无冲突子任务真实并行；同 Device/PTY/Output 的子任务稳定串行；Job Poll 不重复返回历史输出。

M1 总完成标准：M1a、M1b、M1c 分别有可运行集成测试，任一后续 Slice 失败不破坏前一 Slice 的单工具全量请求基线。

### M2：项目、Research、记忆和缓存

- [ ] `.cyber-agent/` 项目隔离。
- [ ] Session 创建和 resume。
- [ ] Session/Research EventEnvelope、JSONL Event Store 和 Sequence。
- [ ] 项目级 advisory writer lock、JSONL 损坏尾部保存与恢复。
- [ ] Session/Research 纯 Reducer、从头重放和从 Snapshot Sequence 继续重放。
- [ ] 所有已发布 Event Version 的连续 Upcaster Registry、历史 fixture 和只读失败模式。
- [ ] Research Ledger、稳定 Research ID、dedup_key 和 action_fingerprint。
- [ ] TargetSnapshot、ExperimentCapsule 以及 Finding/Experiment 的 Snapshot 引用。
- [ ] `RecordsMerged` 和显式 rerun reason。
- [ ] 显式 export/migrate，不原地重写原始 Event Stream。
- [ ] Project Snapshot、File Manifest 和 Tool Capabilities Cache。
- [ ] Project Memory Core、Notes 和 Session Summary。
- [ ] 关键词/Section 级 Memory Retrieval。
- [ ] Memory 命令和 diff。
- [ ] `memory_read`、`memory_write` 和 `artifact_store` 工具。
- [ ] Blob Store、ArtifactRecord、Artifact Lineage 及大型 Tool Result 引用。
- [ ] 只对满足 Determinism/Effect 条件的 Tool Result 启用可删除 content-addressed Cache。
- [ ] `write_file` 和 Cyber Agent 自有 `apply_patch` Adapter，不依赖 `codex-apply-patch`。
- [ ] 文件编辑的 preimage、正常失败不部分写入、前后 Hash 和审计 Diff 测试。
- [ ] `inspect_project`、`research_get` 和 `research_update`。
- [ ] 第二写进程拒绝、尾部截断恢复、非尾部损坏停止写入和外部 Memory 编辑补记测试。
- [ ] 两项目记忆不串扰测试。

完成标准：中断后通过事件重放恢复会话和研究台账，历史 Schema 可经 Upcaster 读取，删除所有 Cache/Summary 仍可重建，且不加载其他项目记忆；TargetSnapshot/ExperimentCapsule 可定位一次实验的版本和输入，项目未变化时第二次 `inspect_project` 命中快照缓存。

### M3：Context 效率、Tool Exposure、DomainProfile 和图片

- [ ] Context Budget。
- [ ] ContextFragment / ContextPlan 和 Fragment Cache。
- [ ] ContextDecisionManifest、稳定 Reason Code 和 `/context` 选择解释。
- [ ] RequestAuditManifest 和首次出现内容的 content-addressed 审计 Blob。
- [ ] ResearchWorldState Snapshot/Diff。
- [ ] Project Snapshot Diff 注入。
- [ ] Full Logical Manifest、Append-only Candidate 和跨 Turn Delta 顺序。
- [ ] History 归一化。
- [ ] Compaction。
- [ ] Direct/Deferred/Hidden Tool Exposure 和 `tool_search`。
- [ ] 固定 DomainProfile Pack 和 Prompt/Tool 指纹。
- [ ] Domain Prompt Profile 和 Agent Model Profile。
- [ ] AgentModelProfile、VisionMode 和纯函数 Vision Route Resolver。
- [ ] `doctor` 对 Primary/Vision Capability 组合做静态验证。
- [ ] `/prompt inspect`、`/prompt diff`、`/context`、`/research`、`/snapshot` 和 `/metrics`。
- [ ] `view_image`、图片 Artifact 和模型输入。
- [ ] `VisionAnalysisService`、`analyze_image`、VisionRouteDecision 和结构化 VisionObservation。
- [ ] `auto`、`primary_direct`、`secondary_tool`、`disabled` 四种模式矩阵。
- [ ] 大型 Tool Result 截断。
- [ ] Session Summary 本地更新不误清空 Baseline，以及不可追加 Memory/State 变化正确全量回退的测试。
- [ ] 历史 Turn 的 Context Recompile，只生成 Eval 派生结果，不改写原 RequestAuditManifest。

完成标准：长会话可压缩继续，可解释每个上下文片段的来源；未变的 State/Prompt Fragment 不重新渲染和计算；低频工具不占用初始 Tool Schema。

### M4：漏洞研究实战验收

- [ ] 固定 `Pragmatic Vulnerability Researcher` Base Contract，不增加 Personality/Role Registry。
- [ ] 源码审计 Prompt Profile。
- [ ] Web 研究 Prompt Profile。
- [ ] Mobile 研究 Prompt Profile。
- [ ] Binary 研究 Prompt Profile。
- [ ] `ResearchProfile` 语义迁移为 DomainProfile，历史 `prompt:mode:*` 审计继续可读。
- [ ] mapping/hypothesis/experiment/verification/evidence_curation 固定 ResearchPhase、Session continuity 和 Turn-boundary selection。
- [ ] Domain + Phase Tool Exposure 合成、Hidden 终止和 Baseline invalidation。
- [ ] immutable SamplingStepPlan/ResolvedToolPlan、current-Turn DeferredActivationSet 和 exact dispatch binding。
- [ ] 固定 `generic-v1` Prompt renderer identity，保持现有 bytes 且无 Dialect Registry。
- [ ] Tool guidance、精确 input/output schema 和结构化 `pre_execution`/`execution` failure；执行后持久化错误走 App/AgentLoop 边界。
- [ ] 使用本地可控样例执行端到端 Eval。
- [ ] ReplayModelProvider、ReplayToolExecutor 和 TraceFixtureManifest。
- [ ] Mechanical Replay、Context Recompile 和显式 Shadow Model Eval 三种隔离模式。
- [x] ObservationEnvelope 外部工具样例和两个 TargetSnapshot 的 TargetDelta。
- [ ] Replay/Shadow 默认不写 Project Memory/Research，未匹配 Tool Call 不执行真实工具。
- [ ] 建立全量/增量请求、单工具/批量工具、无缓存/有缓存的对照 Eval。
- [ ] 统计模型往返、Token、acceptable first-action、参数失败/纠正、重复读取、Batch、Deferred Search、Snapshot Hit、Memory Retrieval 和重复 Hypothesis；unmatched Replay termination 与 Observation/Replay/Vision promotion 只有 typed durable signal 后才统计，否则保持 Deferred，绝不假 0。
- [ ] `http_request` 和 `inspect_environment` 两个通用研究工具。
- [x] AndroidManifest 外部 Extractor，通过 `run_commands` 复用现有工具链，不在 Rust 内重写解析器。
- [x] typed EvidenceLink、legacy Upcaster 和 pure evidence-gap projection。
- [ ] ContextSlot/Origin/Authority/Retention/Freshness、ContextDeliveryMap 和 Evidence-Spine Summary v2。
- [x] exact identifier + fielded BM25 + CJK/ASCII n-gram + RRF + authority/freshness retrieval Shadow。
- [x] deterministic evidence-gap/NextAction guidance Shadow，无 autonomous execution。
- [ ] 根据 Eval 删除无用抽象和 Prompt。

完成标准：至少完成一个长流程漏洞研究任务，且恢复、记忆、Research Ledger、TargetSnapshot、ExperimentCapsule 和证据引用正常；固定 Trace 可回放核心不变量，版本差分能解释来源；增量、批量和缓存方案在对照 Eval 中减少模型往返或重复 Token，否则删除对应复杂度。

### F2a：正式 CLI Vision Ingress

- [x] `/image <path>` 通过既有 `view_image`/Artifact/Receipt owner 排队一个经过验证的本地图片给下一 Turn。
- [x] Primary 与可选 Vision Provider 通过唯一 Project-aware Composition Root 解析同一 Project 的图片 Artifact。
- [x] `primary_direct` 只向 Primary 交付图片，`secondary_tool` 先持久化严格 VisionObservation 再进入同一 Primary AgentLoop。
- [x] 正式 CLI 同时处理初始 operator 图片和模型 `view_image` Tool follow-up，不静默遗漏或重复发送。
- [x] disabled/无能力/无 TargetSnapshot/损坏或超限图片在 Provider 请求前明确失败。
- [x] 双 Provider 与单多模态 Primary 的固定 DeepSeek Live lanes 分别通过，并保留 Primary/Vision Session、RequestAudit、ContextDeliveryMap、Receipt 和 Evidence 边界。

完成标准：正式 `chat` 完成可审计的 `image -> Direct 或 VisionObservation -> Primary AgentLoop -> Tool/Receipt -> final`，不增加第二 AgentLoop、Provider/Wire binding、图片事实源、通用附件系统、Web/TUI 或新依赖。

### M5：可选界面和构建

- [ ] 评估 CLI 是否已足够。
- [ ] 只在 CLI 明显不足时实现本地 Web feature。
- [ ] 添加默认 `ci.yml`，只运行 fmt、clippy、Mock/Fixture/Replay test 和 macOS arm64 build，不配置 API Secret。
- [ ] 添加手动 `live-smoke.yml`，只从 `live-test` GitHub Environment 向所选 Primary/Vision Live Test step 注入对应 CI 专用 `CYBER_AGENT_*_API_KEY`。
- [ ] 生成校验和且不打包 API Key。

## 21. 测试红线

- 两个项目的 Memory、Prompt、History 或 Artifact 相互串扰。
- Session Summary、ResearchWorldState 或 Cache 反向覆盖 JSONL/Markdown/Artifact 事实源。
- 同一有序事件流多次重放得到不同 State，或同 `event_id` 被 apply 两次。
- 发布两个以上 Schema 版本后旧 Event Stream 无法通过连续 Upcaster 读取，或迁移过程原地覆盖原始 Stream。
- 缺失 Upcaster/未知状态事件被静默忽略后继续写入。
- 同一项目允许两个进程并发写 Event Stream，或 JSONL 损坏后静默跳过/覆盖事件继续写入。
- 删除 Cache、Snapshot 和 Summary 后无法从事实源重建项目状态。
- 默认 FullAccess 却因隐藏 Policy 或沙箱阻断工具。
- 模型 Tool Call 和 Tool Result 无法正确配对。
- Provider-specific HTTP 类型或 `previous_response_id` 进入 Domain/Research/Context 协议，导致更换 Adapter 必须修改 AgentLoop。
- Provider Adapter 读取 `OPENAI_API_KEY`、`~/.codex`、`CODEX_HOME` 或 Codex OAuth/session data 作为隐式回退凭据。
- 默认测试、PR CI 或 Fork 能触发真实 Provider 请求或获得 Live Test Secret。
- 根据模型名称猜测 Capability，或 `auto` 因价格、延迟、故障、输出质量而不可审计地切换模型。
- `primary_direct`/`secondary_tool` 的强制模式在能力不满足时仍发送请求，或图片在 Primary 与 Vision 两条路径中都被静默遗漏。
- Primary 与 Vision Provider 交叉复用 Response ID、Server Output Items、ProviderSession 或 IncrementalBaseline。
- Vision 模型拥有独立 AgentLoop/Research/Memory，或 VisionObservation 未经真实验证就提升为 Finding Evidence。
- 为 Mapping/Experiment/Verification 创建不同人格或 Agent，或把 DomainProfile、ResearchPhase、AgentModelProfile 合并成组合角色。
- ResearchPhase 在 Turn 中途生效、由模型自由文本隐式切换，或另建 Workflow/Phase 状态机覆盖 Research Ledger。
- Prompt Full 顺序缺少 Domain/Phase 稳定身份，或 Tool Specs 被拼接进普通 Prompt。
- Hidden Tool 被 Domain/Phase/`tool_search` 提升，或 Tool Exposure 被实现成权限/审批系统。
- 同一 Sampling Step 的 sent ToolSpecs 与 dispatch runtime 来自不同 plan，执行时回查 mutable Registry，或 Deferred activation 泄漏到下一 Turn/Resume。
- 为一个 `generic-v1` renderer 提前建立 PromptDialect Trait/Registry/配置，或 renderer identity 未进入 Fingerprint/Audit/Baseline。
- 已知 Tool 输入/输出仍用无约束 Schema，Tool description 无法区分使用/禁用/优先/产出语义。
- 参数验证失败产生虚假 ExecutionReceipt，或 execution 失败被 Runtime 自动重试；执行后 Receipt/History/RequestAudit 持久化失败被误报为 ToolFailure，或改写成功 ToolResult/伪造 Receipt。
- 已有完成 Receipt 的 ToolExecutionPlan 因 Provider 重试再次执行，或 `never_auto_retry` 工具被自动重放。
- 非确定性/有外部副作用的工具结果被 content-addressed Cache 静默复用。
- `write_file`/`apply_patch` 校验或应用失败后留下部分写入，或成功后没有前后 Hash 和实际 Diff 可审计。
- 网络重试导致本地工具重复执行。
- 满足兼容条件的同 Turn 后续采样或同进程下一 Turn 仍反复发送全部 Input，且无可解释回退原因。
- 仅因 `summary.md` 在 Turn 结束时更新就清空活跃 Baseline，或把需要替换旧语义的 Context 变化错误当成追加 Delta。
- Resume、Compaction、Model/Profile/Prompt/Tool Schema/ModelInputIR Schema/Capability Mapping 变化后仍复用旧 IncrementalBaseline/Response ID。
- 不影响语义的 internal metadata 导致增量请求随机失效，或影响语义的字段被错误忽略。
- 项目未变化时仍重新遍历和读取全部文件内容。
- 每个 Step 对全部项目文件计算内容 Hash，或仅用 mtime/size 作为长期内容一致性依据。
- 未变的 Prompt/State Fragment 每次重新渲染、Tokenize 或以动态顺序注入。
- 可以一次批处理的多文件读取被拆成多次模型往返。
- 无资源冲突的工具被全局串行，或同一 Device/PTY/Output 的冲突工具被并行。
- Job Poll 在每次请求中重复返回所有历史输出。
- 模型反复创建等价 Hypothesis 或重试已有负向 Experiment，Research Ledger 未能发现。
- Experiment/Finding 没有 TargetSnapshot 仍继承旧版本结论，或 Target 变化后复用旧 action_fingerprint。
- 相同 Blob Hash 导致不同实验中的 Artifact Provenance 被错误合并。
- Observation 的模型置信度未经 `research_update` 就被提升为 Finding/已验证事实。
- Core 仅根据模型相似度自动合并 Research Record，或合并后删除原始记录。
- Compaction 后丢失 Target/Version/事实引用/下一步，或 Summary 文本被当作新的 verified Research authority。
- 项目记忆改动无法在本地审计。
- Prompt/Memory/Schema 源内容变化后，无法从 Event、RequestAuditManifest 和 Artifact 重建历史语义请求。
- Context Planner 无法解释片段包含/排除原因，或离线 Recompile 改写历史 RequestAuditManifest。
- 最终 instruction byte/ContentItem 无法映射到 Fragment/Subitem，provenance 被塞进 Provider-specific ContentItem，或 Provider DeliveryItemId 进入 Research identity。
- legacy evidence 被猜测为 positive/negative/environment control，或 Prompt/Summary/Vision/Replay 自动创建 EvidenceLink。
- 检索跨 Project/Target 泄漏、用一个 opaque score 覆盖 authority/freshness，或未经过 Shadow 证据就进入模型 Context。
- Evidence-gap/Guidance 把 Unknown 当 Missing、自动切换 Phase/执行 Tool/创建 Experiment/提升 Finding，或依赖远程 judge/learned ranker。
- Replay/Shadow 执行真实工具、写入正式 Research/Memory，或其结果被当作真实漏洞证据。
- Tool Accuracy Eval 依赖第二套 Trace/事实库、远程裁判服务、controlled public projection 入口或假 0，或评分结果自动改变正式 Finding/Memory。
- 未经配置即向 Model Provider 以外的产品端点发送数据。
- CLI 必须依赖云 Server 才能运行。
- 为一个内部功能新增 crate、网络服务或通用插件框架，但无真实使用证据。
- `domain/ports/adapters/app/ui` 出现文档禁止的反向依赖。
- Cargo.toml 直接引入 `codex-core`、`codex-tools`、`codex-apply-patch` 或其他已禁止依赖。

## 22. 维护规则

- 详细强制规则见 `docs/development/LEAN_CODE_GOVERNANCE.md`，所有实现 Agent 和经明确授权的开发子 Agent 都必须亲自阅读。
- 新功能先用内部 module 实现。
- 同一语义只有一个所有者；多个调用方复用统一实现，不复制 Capability、错误映射、Tool Pairing、截断、排序、Fingerprint、取消或失效逻辑。
- 通用抽象遵循三次重复规则；唯一性和规范化不变量从第一次实现起只有一个所有者。
- 默认使用 `pub(crate)`；测试访问不是扩大公共 API 的理由。
- 不创建 `common.rs`、`utils.rs`、`helpers.rs`、`base.rs`、通用 `manager.rs` 或 Hook Registry 杂物抽象。
- 不创建 Personality/Role、Prompt Stage、DomainProfile、ResearchPhase 或 Tool Exposure 动态 Registry；固定轴使用显式类型和一个组合路径。
- 不创建 Prompt renderer、Tool selector、Retrieval 算法动态 Registry；一个真实实现用显式版本和固定 owner。
- 稳定人格只属于 Base Contract；DomainProfile 只拥有领域方法，ResearchPhase 只拥有当前目标和阶段 Tool Exposure，AgentModelProfile 只拥有模型组合。
- SamplingStepPlan 捕获后不可变；RequestAudit、sent ToolSpecs、ToolCall validation/dispatch 和 Receipt 复用同一 ResolvedToolPlan。
- Context provenance 使用 ContextDeliveryMap sidecar，Summary 使用 typed Evidence Spine；两者不得复制/提升事实权威。
- 能通过 Prompt、Markdown 模板或外部脚本解决的问题，不新增 Core 抽象。
- 能通过 `run_command` 调用的漏洞研究工具，不重写客户端。
- 每增加一个长期依赖，都记录它解决的具体问题。
- 每个新事件类型、配置项和 Prompt 片段必须有消费方和测试。
- Tool guidance、input/output Schema、结构化失败、Provider 编码和 RequestAudit 必须复用同一个 ToolSpec 所有者。
- 只有 pre-execution 参数失败允许模型用新的 ToolCall 纠正；Runtime 不自动重试，执行后的 Effect/Retry/Receipt 语义不被 Prompt 覆盖。
- `chat`、`doctor` 和 Live Smoke 必须复用唯一 Provider Composition Root；通用运行路径禁止写死 Provider URL 或模型环境变量。
- Provider Adapter 只负责 ModelInputIR 编解码和能力映射；wire-specific 类型不得上浮到 Domain。
- 相同 Wire API 的请求编码、Transport 和响应解码不得因新增 Provider 配置而复制，厂商名称本身不构成 binding。
- 每个 Tool Cache 必须证明 Tool Version、TargetSnapshot、Input、Environment 和 Effect/Determinism 都进入 Key。
- 每个 Artifact 派生操作必须保留 Parent、Transform、Producer 和 TargetSnapshot，Lineage 只作为派生视图。
- Replay、Observation 和 TargetDelta 优先使用现有 Event/Artifact，不新增权威数据库。
- Retrieval/Guidance 先在 local Shadow 运行；candidate eligibility 与 ranking 分离，报告不写正式 Research/Memory。
- 每个性能缓存必须有稳定 Key、明确失效条件、Hit/Miss 指标和删除重建路径。
- 优化功能必须通过对照 Eval 证明减少模型往返、重复 Token、重复 I/O 或工具等待，否则不保留。
- `src/` 只保存生产代码；永久回归测试保留在顶层 `tests/`，临时就地测试必须在 Gate 前迁移或删除，发现的回归不得因测试已通过而删除。
- 每个 Slice 在 STATUS/PR 中记录 Lean Review：语义所有者、复用点、新 `pub`/依赖、测试布局、Codex 分类、删除/Deferred 项和治理债务。
- 每个里程碑结束后删除未被真实工作流使用的代码。
- 保持一条命令可运行：`cargo test`。
- 保持一个默认二进制文件，不发布一组必须协同部署的服务。

## 23. 已确定决策

- [x] 新建专用项目，不裁剪完整 Codex。
- [x] 单人使用和维护。
- [x] 单 Rust package、单二进制文件。
- [x] 单 crate 内使用 `domain/ports/app/adapters/ui` 单向依赖。
- [x] 默认 FullAccess。
- [x] 项目级隔离。
- [x] Markdown + TOML + JSONL + Artifact，MVP 不使用 SQLite。
- [x] 使用轻量 Research Ledger，不使用强制 Finding/Validator 平台。
- [x] Session/Research 使用带 Schema Family/Version 的 EventEnvelope 和纯 Reducer，Summary/Snapshot 不是事实源。
- [x] Event Schema 使用连续 Upcaster 链长期读取，缺失迁移时只读失败，原始 Stream 不原地改写。
- [x] Event Stream 使用项目级单 Writer 锁；仅自动恢复损坏尾部，非尾部损坏停止可写模式。
- [x] Research 使用确定性 dedup_key/action_fingerprint，语义相似不触发自动合并。
- [x] 使用 TargetSnapshot + ExperimentCapsule 固定目标版本、环境、输入 Artifact 和 ExecutionReceipt。
- [x] Blob 与 Artifact 身份分离，Lineage 从 Event/ArtifactRecord 派生，不引入图数据库。
- [x] 保留 ResearchWorldState Snapshot/Diff，但只包含漏洞研究 Section。
- [x] `previous_response_id` 增量请求进入 M1。
- [x] 同进程、同 Session 可跨 Turn 复用 IncrementalBaseline；Resume/进程重启/Compaction/Profile/Prompt/Tool Schema/ModelInputIR Schema/Capability Mapping 变化时清空，不跨进程复用 Response ID。
- [x] 保留 Direct/Deferred/Hidden Tool Exposure。
- [x] 使用批量工具和 Resource Key 并发，不使用多 Agent 换取吞吐。
- [x] ToolSpec 增加 Effect/Retry/Determinism/Cache 语义，仅用于执行可靠性和效率，不增加审批。
- [x] Project Snapshot、Context Fragment 和 Memory Retrieval 缓存必须可删除重建。
- [x] Project Snapshot 使用便宜指纹 + 按需 BLAKE3 的两级失效，Artifact Blob 使用 SHA-256。
- [x] M1 拆分为全量单工具、增量 Provider Session、批量/Job/Resource Key 三个可独立交付切片。
- [x] 不直接依赖 `codex-core`、`codex-tools`、`codex-apply-patch` 和完整 Codex PTY Runtime。
- [x] 文件编辑使用 Cyber Agent 自有 Tool Adapter，首期只实现 atomic whole-file write、最小统一 Diff 或外部 `git apply`。
- [x] 本地效率指标和对照 Eval 进入 MVP。
- [x] 请求审计使用 Event + 去重 RequestAuditManifest/Artifact Blob；完整原始 HTTP Snapshot 默认关闭。
- [x] Core 使用 Provider-neutral ModelInputIR，OpenAI Responses 编解码只存在于 Adapter。
- [x] ContextDecisionManifest 记录片段选择 Reason Code，支持不改写历史的离线 Recompile。
- [x] Trace Replay/Shadow Eval 作为隔离的 P1 验证能力，不能产生真实漏洞证据。
- [x] ObservationEnvelope/TargetDelta 是外部工具输出和版本差分协议，不是新的事实数据库。
- [x] 保留图片输入。
- [x] CLI 优先，Web 可选。
- [x] 不实现多 Agent、团队、云 Server 和插件市场。
- [x] 只保留一个 `Pragmatic Vulnerability Researcher` Base Contract，不实现多人格或角色 Agent。
- [x] DomainProfile、ResearchPhase、AgentModelProfile 正交组合，不建立 Domain × Phase 角色或 Workflow Engine。
- [x] ResearchPhase 固定为 mapping/hypothesis/experiment/verification/evidence_curation，只在 Turn 边界切换；首版默认 Mapping 并接受显式 operator 选择。
- [x] Tool 准确率由 guidance/schema/exposure/runtime validation 和 Replay/Shadow Eval 共同保证，不依赖 Prompt 人设单独保证。
- [x] 每个采样 Step 使用 immutable SamplingStepPlan/ResolvedToolPlan；Deferred activation 只在当前 Turn 后续 Step 生效。
- [x] 当前 Prompt renderer 固定为 `generic-v1`，没有第二实现前不增加 Trait/Registry/Config。
- [x] Context provenance 使用 sidecar；Fragment/Subitem Domain identity 与 Adapter DeliveryItemId 分离。
- [x] Research evidence 使用显式 typed relation；Session Summary v2 使用 Evidence Spine + lossy Narrative，不复制第二事实源。
- [x] Security Retrieval 与 evidence-gap/NextAction guidance 使用 deterministic Shadow-first 方案，不默认使用 vector/graph DB 或 remote judge。
- [x] 不上报日志、telemetry 和 feedback。
- [x] 文档基线先于代码 Slice 发布；实现阶段继续按单 Slice 授权和 Gate 推进。

## 24. 待实现时确定

- [x] Rust MSRV 固定为 `1.93`，工具链固定为 `1.93.1`。
- [ ] CLI 行编辑库。
- [x] OpenAI Responses 使用精简自建 Adapter：`reqwest` + `eventsource-stream`，不依赖 Codex/Provider SDK；见 ADR-0003。
- [ ] Token 估算库。
- [ ] WebSocket 连接复用是否能显著降低延迟。
- [ ] Deferred Tool Exposure 的 Schema Token/工具数量启用阈值。
- [ ] M4-S3f Shadow 的首个 recall/precision、重复读取/Token promotion 阈值，以及是否有实测延迟证明需要可重建 SQLite FTS。
- [ ] `fast` Profile 是否自动处理 Compaction 和 Memory 更新。
- [ ] Tool Result Cache 对哪些确定性工具实际有收益及启用阈值。
- [x] 第一批 Observation Kind 和外部 Extractor Schema：严格 v1 `android_manifest_component` Observation batch，绑定 `run_commands`、真实 Receipt、原始 Artifact、TargetSnapshot 和 `extractor_id@version`。
- [x] 第一个 TargetDelta 样例：Mobile / AndroidManifest component、exported、permission 和 intent-filter 差分。
- [ ] Shadow Model Eval 的固定 Trace Suite 和成本上限。
- [ ] 是否有足够现有 Research Ledger 字段支持纯函数自动 Phase projection；在此之前保持默认 Mapping + 显式选择。
- [ ] Tool Accuracy controlled cases 的 acceptable first-action 集合和首个基线阈值。
- [ ] Evidence-gap/Guidance controlled cases 的关键 gap closure、成本和 mutation-risk 基线阈值。
- [ ] 第一个端到端漏洞研究 Eval。

这些问题在对应里程碑开始时决定，不提前搭建抽象。

## 25. 下一步

F3 当前更新：M5-S1a/S1b/S2 已完成并远端封存；F3 已通过独立 Terra xhigh 审计、完整离线 Gate（406 passed / 2 Live ignored）、Lean Review、正式 CLI 合成 Live 和本地分发验证，并经 PR #67 / Squash Merge `5873c08` 远端封存。Full-only 活跃推理遇核心上下文替换或无法保留的预算时仍显式停止，Resume／显式压缩后恢复；不能宣称无缝热替换。非交互执行与 benchmark 适配暂不实现，下一 Slice 未授权。以下为此前 milestone 封存时的历史交接，不是当前 Slice 授权；当前状态仅以 STATUS 为准。

具体状态始终以 `docs/development/STATUS.md` 为准。F1a-F1e 已远端封存；F1e 的远端 Gate 由用户在 GitHub 账单/额度导致零 Step 失败后明确单次豁免，后续 Actions 暂停。Architecture v1.2 至 v1.6 的远端架构封存仍未授权。

M4-S3e1 已通过独立 Terra xhigh 审计、完整 Gate，并经 PR `#52`/Squash Merge `7d5b6d0` 远端封存。M4-S3e2 provenance/Evidence-Spine Compaction 已完成独立 Terra xhigh 审计、完整本地 Gate、合成 DeepSeek Live Tool Loop、Lean Review，并经 PR `#53`/Squash Merge `1f382ee` 远端封存。M4-S3f deterministic retrieval Shadow 已完成独立 Terra xhigh 审计、修复、完整本地 Gate 与 Lean Review，并经 PR `#55`/Squash Merge `02bdbaa` 远端封存。M4-S3g deterministic guidance Shadow 已完成独立 Terra xhigh 审计、修复、完整本地 Gate 与 Lean Review，并经 PR `#57`/Squash Merge `5f63157` 远端封存。M4-S4 真实长流程验收已完成 S3e2 回归审计、受控离线长流程、独立修复审计、完整本地 Gate、显式 DeepSeek text-only Live lanes 与诚实的 Vision/multimodal `UNAVAILABLE` 记录，并经 PR `#59`/Squash Merge `a7143ba` 远端封存；后续双 Provider 协议/服务兼容结果经 PR `#61`/`385a318` 补充封存。F2a 正式 CLI Vision ingress 已完成独立审计、完整 Cargo/Lean Gate、双 Provider Secondary 和单多模态 Primary Direct Live 验收，并经私有 `[skip ci]` PR `#62` 远端封存；当前停止，M5-S1 保持 `READY_NOT_AUTHORIZED`。
