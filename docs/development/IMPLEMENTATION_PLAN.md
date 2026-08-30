# Cyber Agent 可执行开发计划

## 0. 适用范围

本计划将 Architecture v1 基线、v1.1 能力驱动模型组合、v1.2 Research Control Plane、v1.3 Immutable Sampling/Evidence-Guided Context 和 v1.4 ToolFailure/准确率可观测性增量转成可由 Codex Luna xhigh 或其他高推理 Codex 实现的 Vertical Slices。

它约束实现顺序、交付物、测试和停止条件，但不授权当前会话自动初始化 Git、提交、推送、创建 PR 或开始 Rust 实现。具体写操作仍由用户当前指令授权。

## 1. 执行器假设

- 执行器标签：`Codex Luna xhigh`，按用户指定名称记录。
- 不依赖该标签的未公开能力；所有要求必须能由普通仓库 Agent 按文件、命令和测试完成。
- 默认由单 Agent 执行；只有当前用户明确授权开发委派时才能使用子 Agent，并且所有子 Agent 共享同一 Slice/Gate。产品运行时仍保持单 AgentLoop。
- 默认可以在授权后的开发会话中运行本地命令和修改仓库文件。
- 开发 Agent 必须服从根目录 `AGENTS.md`。

## 2. 每次开发会话的固定入口

开始任何 Slice 前：

1. 阅读 `AGENTS.md` 全文。
2. 阅读 `docs/development/STATUS.md` 全文。
3. 阅读本文件中当前 Slice 全文。
4. 阅读 `docs/development/LEAN_CODE_GOVERNANCE.md` 全文。
5. 阅读 `task.md` 相关章节、测试红线和维护规则。
6. 阅读冻结架构 v1/v1.1/v1.2/v1.3/v1.4 对应接口章节及当前 Slice 的 ACCEPTED ADR。
7. 检查当前工作树、分支、最近提交和已有测试。
8. 确认没有修改 `codex/`，没有覆盖用户未提交内容。
9. 将 STATUS 的当前 Slice 标为 `IN_PROGRESS` 后才开始代码修改。

如果 STATUS 与代码事实冲突，以代码和测试为事实，先修正 STATUS，不重复已完成工作。

## 3. Slice 执行协议

每次只执行一个 Slice：

1. Scope Lock
   - 明确 Slice ID、目标、允许修改的模块和禁止事项。
   - 声明新语义的唯一所有者、新 `pub` 契约、长期依赖、Codex 复用分类和预期删除项。
   - 不顺手实现下一 Slice。

2. Contract First
   - 先定义或确认 Domain Type、Port、错误语义和测试不变量。
   - 不先写大 Adapter 再倒推协议。

3. Thin Vertical Path
   - 先让最短真实路径编译、运行和测试。
   - 再补失败路径、恢复、审计和边界测试。

4. Verification
   - 运行当前 Slice 命令。
   - 运行此前全部 Slice 的回归测试。
   - 运行 Lean Gate，检查就地测试、重复语义、公共 API、依赖、消费者、删除项、禁止依赖和文档漂移。

每个 Slice 的固定顺序必须精确为：专项测试 → 受影响测试 → 独立审计 → 修复 → 完整 Gate → Lean Review。审计必须限时；超时或无结论不记为 `APPROVE`。实现者不得审计自己的变更，跨 Port/Adapter 的最终审计由 Terra xhigh 或主 Agent 完成。

5. Handoff
   - 更新 STATUS：完成项、测试结果、Lean Review、剩余风险、下一 Slice。
   - 如果授权 Git 提交，按 Git 方案提交；否则保留工作树并报告。
   - 停止，不自动进入下一 Slice。

## 4. 全局 Definition of Done

一个 Slice 只有同时满足以下条件才完成：

- 目标行为存在可运行测试，不只存在类型或 TODO。
- `cargo fmt --all -- --check` 通过。
- `cargo clippy --locked --all-targets --all-features -- -D warnings` 通过。
- `cargo test --locked --all-targets --all-features` 通过；尚无 feature 时使用对应精简命令。
- `cargo build --locked` 通过；M0 创建 Cargo.lock 后始终使用 `--locked`。
- 没有修改 `codex/`。
- 没有直接依赖禁用 Codex crate。
- 没有未解释的新长期依赖。
- 每个语义只有一个所有者，没有复制 Capability、错误映射、Tool Pairing、截断、排序、Fingerprint 或取消逻辑。
- 新 `pub` 契约有真实跨边界消费者；测试访问不是扩大公共 API 的理由。
- 没有为未来 Slice、测试访问或绕过失败而创建的抽象、Feature、配置或 TODO 框架。
- 没有把 Cache/Summary/Replay 结果变成事实源。
- 新事件、Schema、配置和 Prompt 有消费者与测试。
- 稳定人格、DomainProfile、ResearchPhase 和 AgentModelProfile 保持正交，没有组合角色、第二状态机或动态 Registry。
- Tool guidance、输入/输出 Schema、结构化失败和阶段 Exposure 由唯一所有者生成并进入 RequestAudit/Fingerprint/Eval。
- 每个 sampling Step 的 Context、sent ToolSpecs、RequestAudit 和 dispatch 绑定同一 immutable SamplingStepPlan/ResolvedToolPlan；Deferred activation 不跨 Turn/Resume，且准确率报告必须由相邻 RequestAudit ToolSpec transition 证明。
- Prompt renderer 保持单一 `generic-v1`；Context provenance 使用 sidecar，Provider DeliveryItemId 不进入 Domain/Research identity。
- Summary/Evidence/Retrieval/Guidance/Accuracy 派生内容不提升事实；Accuracy 只报告持久 Trace 可诚实观测的指标，unmatched Replay termination 和 Observation/Replay/Vision promotion 在无 typed durable signal 前为 Deferred，绝不假 0；Shadow 优化在达到明确 promotion gate 前不改变模型输入或自动动作。
- `src/` 不含就地 `#[cfg(test)] mod tests`，永久回归测试保留在顶层 `tests/`。
- 无消费者代码、重复 Fixture 和无收益优化已删除或明确 Deferred；测试不得通过新增 controlled public projection 入口绕过真实生产边界。
- STATUS 记录本 Slice Lean Review。
- STATUS 与实际代码状态一致。
- 如果修改架构契约，已有 ADR 和新架构版本计划。

## 5. 测试层次

所有永久测试位于顶层 `tests/`。`src/` 只保存生产代码；临时就地测试只能存在于未提交探索过程，Gate 前必须迁移或删除。发现缺陷或不变量时，先转成永久回归测试再删除临时版本。测试共享构造器放在 `tests/support/`，稳定样本放在 `tests/fixtures/`，不得复制生产逻辑或同一 Fixture。

测试布局、迁移和 inline-test 静态检查以 `docs/development/LEAN_CODE_GOVERNANCE.md` 为准。

### 5.1 Domain Unit

- Reducer、Upcaster、Fingerprint、Dedup、Context Decision 和失效矩阵使用纯测试。
- 固定输入必须产生确定结果。
- 时间、随机 ID、文件和网络通过参数或 Port 注入。

### 5.2 Port Contract

- 每个 Adapter 必须通过相同 Contract Test。
- Model Provider 测试事件顺序、取消、Usage、Tool Call 和能力错误。
- Tool Adapter 测试 ExecutionPlan/Receipt、Resource Key、超时和输出策略。
- Event Store 测试顺序、尾部恢复、锁和迁移。
- Artifact Store 测试 SHA-256 去重与 Provenance 不合并。

### 5.3 Integration

- Mock Provider + 真实本地文件工具完成 AgentLoop。
- 通用 Provider Live Smoke 默认 ignored，只有 `CYBER_AGENT_LIVE_TESTS=1` 且 `CYBER_AGENT_LIVE_CONFIG` 指向显式 Config v1 时才能开启；所选 Provider 必须具有本地内联 `api_key` 或可精确解析的专用 `credential_ref`。
- 默认测试不能依赖公网、真实设备或用户 API Key。
- Live 测试只使用固定合成输入，不读取真实项目 Memory、Session、Artifact 或漏洞目标。
- 凭据 Contract Test 使用 Fake Credential Source，证明不会读取 `OPENAI_API_KEY`、`CODEX_HOME`、`~/.codex` 或 Codex OAuth/session data。

### 5.4 Golden/Fixture

- Event 每个历史 Schema 至少一个 fixture。
- ModelInputIR 与 Responses wire 编码使用 snapshot fixture。
- ContextDecision Reason Code 使用稳定 golden。
- Trace Replay 使用固定 Event/Artifact fixture。

## 6. 里程碑和 Slice

### M0-S1：仓库与单 Package 骨架

目标：建立最小可编译二进制和开发约束。

交付：

- `Cargo.toml`
- `Cargo.lock`
- `rust-toolchain.toml`
- `src/lib.rs`
- `src/main.rs`
- CLI `--help`、`init`、`chat`、`doctor` 空壳
- `.gitignore`

要求：

- 单 package，不创建 workspace。
- 只引入 CLI、Serde、错误类型和异步运行所必需依赖。
- `main.rs` 只做参数解析和 Composition Root 占位。

测试：

- 二进制 `--help` smoke test。
- `doctor` 在无配置时返回结构化诊断而不是 panic。

Gate：本地 fmt、clippy、test、build 全部通过。

### M0-S2：Domain 边界与 Event 最小契约

目标：建立不会被后续 Adapter 反向污染的内核类型。

交付：

- `domain/events.rs`
- `domain/model.rs`
- `domain/tools.rs`
- `domain/context.rs`
- `ports/` 最小 trait
- EventEnvelope Family/Version
- 当前版本 Upcaster identity path
- 空 State 的纯 Reducer

测试：

- Event 序列确定性。
- 重复 event_id 不二次 apply。
- Domain 不依赖 app/adapters/ui 的结构检查。

Gate：没有 I/O 进入 Domain。

### M1a-S1：ModelInputIR 与 Mock Provider

目标：在无网络条件下建立完整模型事件流契约。

交付：

- ModelInputIR、ContentItem、ModelEvent、显式且版本化的 ModelCapabilities。
- ModelProvider Port。
- MockModelProvider。
- Streaming 消费与取消的最小 App Service。

测试：

- Text、Tool Call、Usage 和完成事件顺序。
- 不支持 ContentItem 的明确能力错误。
- Provider-specific 类型不出 Adapter。
- Capability 不按 Model slug 推断；text/image/streaming/tool/structured/incremental/usage 使用显式 Fixture。

### M1a-S2：OpenAI Responses 全量 Streaming

目标：实现一个最小 OpenAI Responses HTTP Streaming Adapter，只发送 Full Request。

依赖和 Transport 决策遵循 `docs/adr/ADR-0003-minimal-openai-http-sse-stack.md`。

当前治理迁移前置条件：

- 在本 Slice Gate 前，将 `src/lib.rs`、`src/domain/model.rs` 和 `src/domain/events.rs` 中现有就地测试迁移到顶层 `tests/`，保留原行为断言。
- 测试迁移不得扩大生产公共 API，不得删除已验证不变量，也不得开始 M1a-S3。
- 运行 `LEAN_CODE_GOVERNANCE.md` 定义的 inline-test 静态检查。

交付：

- OpenAIResponsesAdapter。
- 稳定 ModelInputIR 编码。
- SSE/Streaming 解码、错误映射、取消和 Usage。
- Adapter/Composition Root 内可注入的 Credential Resolver；Domain 和 App 不持有 Provider Secret。
- MVP 默认只通过 `CYBER_AGENT_OPENAI_API_KEY` 读取 API Key；Keychain Adapter 属于后续可选增强，不阻塞 M1a。
- 凭据只在显式 live Provider 路径按需解析；Mock、Fixture、Replay 和普通测试不读取 Key。

测试：

- 编解码 fixture。
- HTTP mock server 的成功、错误、断流和取消测试。
- Live smoke test 为 ignored/opt-in。
- Credential Resolver 使用 Fake Source 的存在、缺失和错误测试。
- 通用 `OPENAI_API_KEY` 已设置但 Cyber Agent 专用变量未设置时，必须得到“未配置”，不能隐式使用通用变量。
- 不存在访问 `~/.codex`、`CODEX_HOME` 或调用 Codex CLI 的代码路径。

禁止：

- 此 Slice 不实现 previous_response_id。
- 不引入完整 Codex Model Client。
- 不实现 Codex/ChatGPT OAuth、登录迁移或凭据导入。
- 不把真实 Key 写入 `.env`、Fixture、Snapshot、Event、Artifact 或测试输出。

### M1a-S3：单工具 AgentLoop

目标：完成 Tool Call -> Tool Result -> Full Resample。

交付：

- ToolSpec、ToolCall、ToolResult。
- ToolExecutionPlan/ExecutionReceipt 最小类型。
- 最小 Tool Registry。
- read_file、list_files、search_files、run_command。
- 内存 History 及 Tool Pairing。

测试：

- Mock Provider 请求工具后得到结果并继续采样。
- Tool Call/Result 丢失、重复、取消和失败配对。
- `run_command` 默认 interactive/never_auto_retry。

Gate：在临时项目目录完成一次真实文件分析闭环。

### M1b-S1：ProviderSession 与 Canonical Baseline

目标：实现稳定请求比较，不发送增量请求。

交付：

- TurnContext、StepSnapshot、ProviderSession。
- IncrementalBaseline。
- Canonical Input、Tool Schema、Output Contract 和 Request Properties Hash。
- 完整失效 Reason Code。

测试：

- Model/Profile/Prompt/Tool/Output/IR/Capability 变化矩阵。
- internal metadata 不导致随机失效。
- History 修改、删除和重排不可视为 append-only。

### M1b-S2：previous_response_id 增量与回退

目标：同进程 Session 内跨采样和跨 Turn 追加 Delta。

交付：

- OpenAI Adapter previous_response_id 映射。
- append-only candidate。
- Provider reject 后最多一次 Full fallback。
- Restart/Resume 不复用旧 Response ID。

测试：

- 同 Turn Tool 循环增量。
- 下一 Turn User Input 增量。
- Compaction/Resume 全量重置。
- Provider reject 回退，不重复本地 Tool。

### M1c-S1：Batch 与 Resource Scheduler

目标：减少模型往返并允许无冲突任务并发。

交付：

- read_files、multi-pattern search_files、run_commands。
- Resource Key resolver 和冲突锁。
- Effect/Retry/Idempotency 解析。

测试：

- 不同文件/设备/输出并行。
- 同 Device/PTY/Output 串行。
- Batch 部分失败不丢其他结果。

### M1c-S2：普通后台 Job

目标：先完成非 PTY 的长运行任务。

交付：

- Job ID、状态、输入、停止。
- stdout/stderr offset poll。
- 超时、取消和完成 Receipt。

测试：

- Poll 只返回新字节。
- Cancel 后不遗留错误状态。
- 完成 Receipt 防重复执行。

### M1c-S3：最小 PTY

目标：支持调试器、Frida、ADB shell 等交互工具。

交付：

- 基于 portable-pty 的当前平台最小 Adapter。
- PTY Session、input、resize、poll、stop。

禁止：

- 不移植 Codex 完整 PTY Runtime。
- 不提前实现跨平台兼容全集。

### M2-S1：项目隔离与 Event Store

目标：把内存状态变成可恢复本地状态。

交付：

- `.cyber-agent/` 初始化。
- Session/Research JSONL Writer。
- project advisory lock。
- 尾部恢复 Artifact。
- Upcaster Registry 和 export/migrate 骨架。

测试：

- 两项目隔离。
- 第二 Writer 拒绝。
- 损坏尾部恢复、非尾部损坏只读失败。
- 历史 fixture chained upcast。

### M2-S2：Research、TargetSnapshot 与 Capsule

目标：完成可复现研究事实源。

交付：

- Hypothesis/Experiment/Finding/NextAction Event。
- TargetSnapshotRecord、ExperimentCapsuleRecord。
- dedup_key、action_fingerprint、RecordsMerged。
- research_get/research_update。

测试：

- Target 语义变化产生新 Snapshot。
- 工具版本变化不伪造 Target 变化，但改变 action fingerprint。
- confirmed Finding 必须引用 Snapshot 和 Evidence。
- 模型相似不自动合并。

### M2-S3：Blob、ArtifactRecord 与 Lineage

目标：保存大型内容并保持来源。

交付：

- SHA-256 Blob Store。
- ArtifactRecord/Event。
- parent/transform/producer/target 关系。
- artifact_store 工具。

测试：

- 同字节只保存一个 Blob。
- 同 Blob 的两次来源保留两个 ArtifactRecord。
- 删除 Cache 不破坏 Artifact/Research 引用。

### M2-S4：Memory、Project Snapshot 与可重建 Cache

目标：项目恢复时不重复扫描并保留长期知识。

交付：

- Memory Core/Notes/Failed Paths。
- memory_read/memory_write 和外部编辑检测。
- ProjectFingerprint 两级失效。
- Project Snapshot/File Manifest/Tool Capability Cache。
- inspect_project。

测试：

- 未变项目命中快照。
- 变化文件按需 BLAKE3。
- Cache 删除可重建。
- 两项目 Memory 不串扰。

### M2-S5：文件写入与确定性 Tool Cache

目标：完成可审计写入和受约束结果缓存。

交付：

- write_file atomic write。
- 自有 apply_patch Adapter。
- preimage、前后 Hash 和实际 Diff。
- deterministic/read_only Tool Result Cache。

测试：

- 校验失败不部分写入。
- non-deterministic/external mutation 不缓存。
- Cache Hit 产生新 Event 并引用旧 Receipt。

### M3-S1：Context Planner 与 Decision Manifest

目标：建立可解释、可预算、可增量的上下文编译器。

交付：

- ContextFragment/ContextPlan。
- Full Logical Manifest/Append-only Candidate。
- ContextDecisionManifest 和 Reason Codes。
- Fragment Cache。
- `/context`。

测试：

- 固定组装顺序。
- include/truncate/artifact/exclude 决策。
- 未变 Fragment 不重渲染。
- Context Recompile 不改写历史审计。

### M3-S2：WorldState、Tool Exposure 与 Mode

目标：只向模型暴露当前研究所需状态和工具。

交付：

- ResearchWorldState Full/Diff。
- Project Snapshot Diff。
- Direct/Deferred/Hidden Exposure。
- tool_search。
- Mode Pack 和 Prompt/Model Profile（M3-S2 的历史实现名；v1.3 在 M4-S3a2 将领域语义迁移为 DomainProfile）。
- AgentModelProfile：一个 Primary、可选 Vision 和 VisionMode。
- `auto`、`primary_direct`、`secondary_tool`、`disabled` 的纯 Vision Route Resolver。
- `doctor` 的 Provider/Mode Capability 组合检查。

测试：

- Deferred Tool 载入导致 Baseline Reset。
- Schema Token 指标。
- 不同历史 Mode 的 Prompt/Tool 指纹稳定；M4-S3a2 迁移后继续验证 Domain/Phase 指纹和历史 Replay。
- Capability 矩阵穷举、无模型名猜测、无价格/延迟/失败驱动的隐式 fallback。
- Profile/VisionMode 只在 Turn 边界切换，必要时重置 Primary Baseline。

### M3-S3a：History、Session Reducer 与 Resume

目标：先建立长会话恢复所需的最小事实和归一化边界，不同时引入 Summary、Audit Blob 或 CLI 组装。

交付：

- `domain/history` 继续作为唯一 History Normalizer 和 Tool Call/Result 配对所有者。
- 复用 `cyber-agent.event` envelope v1，为 Session payload 定义独立、严格、版本化的 schema v1 和 Reducer。
- Session Event 只有在 durable append 成功后才进入 Reducer。
- Resume 创建全新的 Primary ProviderSession，丢弃旧 continuation/Incremental Baseline，首次真实请求以 `SessionResumed` 原因发送 Full。

测试：

- 完整 Tool Pair 保序；孤立 Result 被删除；未完成 Call 获得稳定 `aborted` Result；重复或错配 ID 被拒绝。
- 未知 Session Event、未知 payload schema 和非法状态转移明确失败。
- append 失败不推进 Reducer；重放严格复原 Session 状态。
- Resume 后新旧 ProviderSession ID 不交叉，首次请求为 Full，成功提交后才可恢复增量。

### M3-S3b：Session Summary 与 Compaction

目标：在不覆盖 Session Event 事实的前提下生成可重建的长会话派生视图。

交付：

- 版本化结构化 SessionSummary 及其从严格 Session Reducer 的确定性投影。
- Compaction 记录 Summary revision/hash、保留边界和归一化后的最近 History；原始 Events 永不删除或覆盖。
- 活跃 Baseline 中的本地 Summary 更新不触发替换；Compaction 后下一次真实请求以 `CompactionReset` 发送 Full。

测试：

- Summary 可从 Events 删除重建，且不能覆盖 Research、Memory 或原始 Session History。
- Compaction 保留完整 Tool Pair、当前目标、Target/Version、已验证事实、活跃假设、失败路径、Artifact、修改文件、未完成执行和下一步。
- 普通 Summary 更新不误清空活跃 Baseline；Compaction 强制一次 Full，成功后恢复增量。

### M3-S3c：Request Audit 与历史语义重建

目标：即使 Prompt、Memory 和项目文件随后变化，也能从本地事实重建历史语义请求。

交付：

- 一个版本化 RequestAuditManifest，按发送顺序记录语义组成、来源、版本、Hash 和内容引用。
- 复用现有 `ArtifactStore::publish_bytes` 和 SHA-256 Blob 身份保存首次出现的渲染内容，不建立第二个 Blob Store 或 Audit Manager。
- 记录 ModelInputIR hash/schema、Provider Adapter version、Capability Mapping hash、delivery mode/reason；排除 API Key、Authorization Header 和非语义 Transport metadata。
- Event durable append 成功后才把一次请求审计视为已提交；提供按 Manifest 重建历史语义请求的只读路径。

测试：

- 相同内容跨请求去重，不同项目的事件引用不串扰。
- 源 Prompt/Memory 修改后仍能按原顺序和原内容重建历史请求。
- Blob 缺失、Hash 不符、未知 schema 或乱序引用明确失败。
- Blob/Event 持久化失败不产生已成功审计的假象。

### M3-S3d：最小检查 CLI

目标：把现有 Context、Snapshot 和审计信息接到个人可用的只读 CLI 检查面。

交付：

- `/prompt inspect` 显示最近已提交请求的组装顺序、来源、版本、Hash、Blob/Event 引用和 Token 估算。
- `/snapshot` 显示 Hit、Incremental Refresh 或 Full Rebuild 及稳定原因。
- `/metrics` 显示当前 Session/项目的本地请求、Token、Full/Incremental、回退、Cache 和 Compaction 指标。
- 复用现有 ContextDecisionManifest、Project Snapshot、Event 和 RequestAudit 事实，不创建第二套指标或状态存储。

测试：

- CLI 对空项目、无请求、损坏引用和正常 Session 均输出稳定且可审计的结果。
- 检查命令只读，不触发 Provider 请求、工具执行、状态推进或凭据解析。
- 输出不包含 credential、Authorization 或非语义 Transport metadata。

### M3-S4：多模态

目标：本地图片通过可审计的 Direct 或 Secondary 路径进入单 Agent 研究流程。

交付：

- view_image。
- 格式/尺寸验证。
- Budget 驱动缩放。
- Original/Derived Artifact 关系。
- VisionAnalysisService 和 `analyze_image`。
- VisionRouteDecision、稳定 Reason Code 和结构化 VisionObservation。
- 单一多模态 Primary 的 `primary_direct` 路径。
- 文本 Primary + Vision Provider 的 `secondary_tool` 路径。
- Mechanical Replay 使用已记录 Observation，零真实 Vision 请求。

测试：

- 支持格式、超限、损坏图片。
- Provider 无图片能力时明确失败。
- 原图 Hash 和派生元数据可审计。
- 四种 VisionMode × Primary/Vision Capability 的穷举矩阵。
- `primary_direct` 证明未调用 Secondary；`secondary_tool` 证明 Primary 首次采样前已有 Observation。
- Primary/Vision Session ID、Response ID 和 Baseline 不交叉。
- VisionObservation 保留不确定性，不能直接成为 Finding Evidence。
- 初始图片不会在两条路径中同时缺失，也不会重复发送。

### M4-S1：漏洞研究 Mode 与可控样例

目标：建立 source/web/mobile/binary Prompt Profile 和离线样例。

要求：

- 样例必须本地可控，不依赖未授权目标。
- Candidate、Reproduced、Confirmed 语义区分。
- 正向、负向和篡改/环境对照保留。

### M4-S2：Trace Replay 与 Shadow Eval

目标：架构和 Prompt 变更可以回归。

交付：

- TraceFixtureManifest。
- ReplayModelProvider/ReplayToolExecutor。
- Mechanical Replay、Context Recompile、Shadow Model Eval。

测试：

- Replay 零真实外部动作。
- Unmatched Tool Call 停止。
- Eval Session 不写正式 Memory/Research。
- Replay 结果不能提升为 Finding Evidence。

### M4-S3a1：不可变 Sampling 与 Tool Plan 基础

目标：先消除一个模型请求内部 Context、Tool Schema 与 Tool dispatch 观察不同状态的可能，再叠加 Research Control Plane。

交付：

- 一个内部不可变 `SamplingStepPlan`，绑定 Step ID、TurnConfigurationSnapshot、ContextPlan、ResolvedToolPlan、AgentModelProfile/Capability Mapping、RequestRequirements 和 Prompt renderer identity。
- Tool Registry 从可变 exposure authority 收敛为不可变 canonical catalog；一个纯 resolver 产生有稳定 fingerprint 的 `ResolvedToolPlan`，同一 Plan 同时提供 sent ToolSpecs 与 model-originated ToolCall dispatch。
- typed `DeferredActivationSet`：`tool_search` 只影响当前 Turn 后续 Steps，下一 Turn 和 Resume 清空；当前 Step 永不原地改变。
- `generic-v1` 固定 Prompt renderer identity 进入 Context/Prompt fingerprint、RequestAudit、IncrementalBaseline 和 Trace；不增加 Trait、Registry 或配置开关。
- 仅做行为保持的最小 `ContextFragmentSource -> ContextSlot + ContextOrigin` 非持久化迁移，为后续 ContextDeliveryMap 留出正交来源边界；不在本 Slice 实现完整 Authority/Retention/Freshness。
- RequestAudit 可绑定 sent ToolSpecs 与 ResolvedToolPlan fingerprint；ToolExecutionPlan/Receipt 绑定产生该 ToolCall 的 Plan identity。

测试：

- 同一 SamplingStepPlan 的 Context、ToolSpecs、RequestAudit 和 dispatch 一致；Plan 捕获后修改候选配置不能影响它。
- Tool Search 成功只改变下一 Step，Turn 结束和 Resume 后消失；Hidden/Unavailable 不可激活。
- 对未广告、Deferred 未激活或 plan fingerprint 不匹配的 ToolCall 返回配对 pre-execution ToolResult，且不查 raw Registry、不执行 Runtime。
- Replay 使用固定 Replay Plan，不能回退 live catalog。
- `generic-v1` 保持现有 Prompt 精确 bytes；renderer identity 变化触发 Full，Primary/Vision identity 不交叉。
- ContextSlot/Origin 迁移保持现有顺序、预算、截断、fingerprint 与历史 RequestAudit/Trace 可读性。

禁止：

- 不实现 stable persona、DomainProfile/ResearchPhase Session Event、完整 ContextDeliveryMap/EvidenceLink/Summary v2、Tool Accuracy、Retrieval、Guidance、新 Tool、Provider 或 Live Test。
- 不复制 Codex ToolRouter/StepContext，不增加第二 Registry/Executor/AgentLoop、Hook、Policy、PromptDialect 或 mutable global exposure。

Gate：完整 locked Cargo/Lean Gate；永久回归覆盖跨 Step/Turn/Resume、dispatch binding、Replay 和历史 audit；证明只有一个 catalog、resolver、renderer、pairing 和 dispatch owner。

### M4-S3a2：稳定研究人格、DomainProfile 与 ResearchPhase

目标：在 M4-S3a1 的不可变 Step/Tool plan 上建立稳定 Agent 身份、正交领域/阶段控制和可审计的 Turn 边界组合。

交付：

- 一个版本化 `Pragmatic Vulnerability Researcher` Base Contract；不增加 Personality enum/registry。
- 将当前语义不准确的 `ResearchProfile` 一次性重命名为固定 `DomainProfile`，保留 source/web/mobile/binary 四项和现有 TargetKind 兼容性。
- 固定 `ResearchPhase = mapping | hypothesis | experiment | verification | evidence_curation`，每项只有稳定 ID、generation、编译期 Markdown Prompt 和 bounded ToolExposurePlan。
- PromptComposition 固定为 Base、Personal、Project、Domain、Phase 后再进入 Research/Project/Memory/History；Tool Specs 仍是独立通道并通过 a1 ResolvedToolPlan。
- Phase 默认/显式 operator 选择、稳定 Reason Code、Session Event/Reducer/Resume 连续性和 Turn-boundary queue/durable-append/apply；append 失败时不发送模型请求。
- 新 fragment identity 使用 `prompt:base:*`、`prompt:domain:*` 和 `prompt:phase:*`；历史 `prompt:mode:*` RequestAudit/Replay 不重写。

测试：

- 恰好一个稳定人格，Domain/Phase 不产生组合角色或第二 AgentLoop。
- 四 Domain × 五 Phase 的 Prompt 顺序、唯一 Context fingerprint 和 Tool exposure 矩阵。
- Phase/Domain 只在 Turn 边界生效；in-flight Turn 不变化；Prompt/Tool 变化触发正确 Baseline reset。
- 新 Session 在首个请求前只记录一次默认选择；legacy Session 只读 Replay 不回填、下一真实 Turn 才追加 Mapping；Resume 恢复最新 Phase；未知/非法 Event 严格失败。
- Hidden 不可搜索/提升；Deferred 使用 a1 的 current-Turn lifecycle。
- 历史 `prompt:mode:*` Trace 仍可 Mechanical Replay/RequestAudit reconstruct。

禁止：

- 不实现模型自由文本阶段切换、自动 Ledger 阶段推断、Workflow DSL、Prompt/Profile Registry、多人格或多 Agent。
- 不同时硬化全部 Tool Schema、不增加 `http_request`/`inspect_environment`、Observation/Extractor/TargetDelta、Provider 或 Live Test。

Gate：完整 locked Cargo/Lean Gate；新增 Event/Prompt/Exposure/Resume/Replay 永久回归；无第二 Prompt/Exposure/Phase owner。

### M4-S3b：Tool 准确率契约与 Eval

目标：让模型以精确 Tool guidance/schema/error 选择并纠正工具，并用现有 Trace 基础量化结果。

交付：

- 所有模型可见 Tool description 使用固定 `Use when`、`Do not use when`、`Prefer`、`Produces` 结构。
- 已知输入/输出使用精确 JSON Schema、required/enum/bounds 和实际 typed result；移除无理由的通用 object output。
- 一个 provider-neutral `ToolFailure`/错误映射所有者，仅区分 `pre_execution`、`execution` 和 retryability；Receipt/History/RequestAudit 等执行后持久化失败属于 App/AgentLoop 错误，不能改写真实成功 ToolResult 或伪造 Receipt。
- 只有执行前参数错误可返回无 Receipt 的纠错结果；任何已开始执行必须有真实 Receipt，模型必须发起新 ToolCall，Runtime 不自动重试。
- 在持久 Trace/Replay/Shadow 上增加 derived non-evidence ToolAccuracyReport 和 tests-only controlled cases；无 typed durable signal 的 unmatched Replay termination 与 Observation/Replay/Vision promotion 保持 Deferred，绝不假 0。

持久 Trace 当前可观测且必须量化：

- acceptable first-action match；
- invalid arguments 和 correction outcome；
- redundant reads 与 missed batch opportunities；
- `tool_search` 调用与命中；
- Tool Schema token cost。

当前必须显式 Deferred：

- unmatched Replay termination；
- Observation/Replay/Vision evidence-promotion boundary violations；
- 不能由同一 Turn 相邻 RequestAudit 精确 ToolSpec transition 证明的 Deferred activation。

RequestAudit 当前没有持久 Turn identity，因此 M4-S3b 不推断 Deferred activation 成功或零；后续只有在既有持久事实能证明同一 Turn，或另一个经 ADR 接纳的真实消费者引入 typed signal 后才可计入。controlled tests 不得新增 public projection 入口。

禁止：

- 不增加工具数量、外部服务、事实库、模型裁判服务或通用 Eval 平台。
- 不删除单数工具来伪造 Batch 指标；先通过 exposure/guidance/Eval 证明取舍。
- 不自动重试已开始、已完成、external-mutation 或 interactive 执行。

Gate：完整 locked Cargo/Lean Gate；所有 Tool Contract/AgentLoop/Responses/Replay fixtures 复用同一 Schema；精确证明参数失败没有 ExecutionReceipt 但仍有配对 ToolResult，执行失败有真实 Receipt 且不会静默重放，执行后 Receipt/History/RequestAudit 持久化失败走 App/AgentLoop 错误边界；未观测指标保持 Deferred 且不假 0。当前 Slice 未通过 Gate 前不得标为 COMPLETE。

### M4-S3c：通用研究工具

目标：补齐高复用网络请求和环境指纹能力，不为厂商或漏洞工具建立专用客户端。

交付：

- `http_request`：结构化 method/URL/header/query/body，bounded timeout/redirect/preview，返回 status/final URL/metadata/timing/body Artifact。
- `inspect_environment`：结构化 OS/runtime、Git、proxy、toolchain 和 connected-device availability/version，加一个稳定 environment fingerprint。
- 两者复用现有 ToolExecutionPlan、Resource Key、ExecutionReceipt、Artifact、Context truncation 和 Tool accuracy 契约。

测试：

- `http_request` 使用 loopback 覆盖成功、4xx/5xx、redirect、timeout、binary/oversize Artifact 和凭据非泄漏；默认 Gate 不访问公网。
- `inspect_environment` 使用注入/临时 Fixture 覆盖存在、缺失、版本失败、稳定排序和 fingerprint；不依赖开发机实际安装集合。
- Phase Exposure、RequestAudit、Tool Result pairing、Cache/Retry 语义保持一致。

禁止：

- 不实现 Web scanner、Provider client、任意 probe DSL、第二 Environment store 或外部漏洞工具 Rust wrapper。

Gate：完整 locked Cargo/Lean Gate；无新增长期依赖，除非实现前有单独 ADR 和不可替代证据。

### M4-S3d：Observation、AndroidManifest Extractor 与 TargetDelta

目标：把外部工具结果导入结构化 Observation，并比较两个目标版本的攻击面。

交付：

- 第一批 provider-neutral Observation Kind 和严格 ObservationEnvelope ingestion。
- AndroidManifest 作为第一个 bounded external Extractor：通过 `run_commands` 调用现有 aapt/JADX/脚本能力，不在 Rust 中重写解析引擎。
- 每项 Observation 绑定 TargetSnapshot、ToolCall、ExecutionReceipt、原始 Artifact、Extractor ID/version。
- 两个 TargetSnapshot/Observation 集合的确定性 TargetDelta。

测试：

- 正常、字段缺失、未知 schema、伪造 Snapshot/Receipt/Artifact 引用和 Extractor version 变化。
- Manifest component/exported/permission/intent-filter 差分稳定排序并可回到原始输出。
- Observation 不能自动成为 Finding；只有显式 `research_update` 可引用提升。

Gate：完整 locked Cargo/Lean Gate；差分每项都能回到 ToolCall、Receipt、Artifact、Snapshot 和 Extractor；无新事实数据库或扫描器框架。

### M4-S3e1：typed EvidenceLink 与证据缺口投影

目标：让支持、反证、控制、可达性、前提和影响证明成为显式 Research 关系，停止依赖 Prompt/摘要字符串猜测证据角色。

交付：

- additive Research payload/version 与 `EvidenceLink { relation, evidence }`，固定 supports/contradicts/positive_control/negative_control/environment_control/reachability_proof/prerequisite_proof/impact_proof。
- chained Upcaster：历史 `evidence_refs -> supports`、`negative_refs -> contradicts`，不把旧引用猜成控制。
- 一个纯 evidence-gap projection，区分 unknown 与 missing，并只读取 typed Research facts。

测试：

- 每种 relation、重复/跨 Project/错误类型/缺失 Artifact 或 Receipt 引用严格验证。
- 历史 fixture 连续 Upcast 且原 Event 不重写；控制关系必须由新显式 Research update 创建。
- Gap projection 不能写 Research/Memory、改变 Phase 或把 Vision/Replay/Eval 提升为证据。

Gate：完整 locked Cargo/Lean Gate；Research schema/upcaster/reducer/reference validation 永久回归；无第二 evidence store 或图数据库。

### M4-S3e2：完整 Context 来源与 Evidence-Spine Compaction

目标：让每个最终请求组件可追溯，并让 Compaction 只引用 Research 事实而不复制“verified facts”权威字符串。

交付：

- 在 a1 ContextSlot/Origin 上增加 SemanticClass、AuthorityLevel、RetentionClass、FreshnessIdentity 和 ContextReferences 校验。
- deterministic ContextSubitemId；ContextPlanner 产生覆盖 instruction spans/content positions 的 ContextDeliveryMap sidecar，记录 content hash 与 truncate/merge/ArtifactRef transform。
- additive RequestAudit schema 保存 map 或 deduplicated Blob reference。
- Session Summary v2 = typed EvidenceSpine + bounded NarrativeSummary；spine 绑定 Research stream identity/watermark/hash，Resume/Rebuild 重验引用。

测试：

- 每个最终 instruction byte/ContentItem 恰好归属一个 Fragment/Subitem，renderer separator 除外；截断/替换仍保留来源。
- legacy unknown provenance 显式存在而不猜测；Provider DeliveryItemId 不进入 Domain/Research。
- Summary v2 缺失、跨项目、stale watermark、类型错配引用失败；删除 derived Summary/map 后事实仍可恢复。
- Summary v1 和旧 RequestAudit fixture 继续可读且语义不被升级。

Gate：完整 locked Cargo/Lean Gate；完整 provenance coverage、Compaction/Resume/Replay 与 schema chain 回归；无第二 Context renderer、事实库或 ContentItem-wide Provider metadata。

### M4-S3f：确定性安全检索 Shadow

目标：用漏洞研究标识符、目标适用性和证据权威度减少重复读取与无关 Context，不先引入向量服务。

交付：

- 一个版本化固定 pipeline：eligibility/Target filter → exact security identifiers → fielded BM25 → CJK/ASCII char n-gram → RRF → authority/freshness tier → stable ID。
- Project Memory、Research、Artifact/Observation metadata 和显式历史查询各自拥有候选 eligibility；只复用已证明的 normalization/ranking primitives。
- local Shadow report 包含 query fingerprint、matched fields/IDs、component ranks、tier、reason、estimated Context cost；不改变 model-visible Context。
- 可删除内存/Cache index；不增加持久索引依赖。

测试：

- CVE/CWE、URL、package/bundle/class/method/symbol、Android permission/intent/component、binary symbol/hash 与 CJK/typo fixtures。
- cross-project 永不候选；stale/cross-target 默认排除；相同输入稳定排序和理由。
- controlled acceptable reference sets 量化 recall/precision、重复读取和 Token；无收益则保持 Shadow 或删除。

Gate：完整 locked Cargo/Lean Gate；Shadow 零 Provider/Tool/Research/Memory 写入；无 selector registry、remote embedding/reranker/judge、vector/graph DB 或 telemetry。

### M4-S3g：证据缺口与实验引导 Shadow

目标：对既有 NextAction/controlled Eval candidates 给出可解释的最小决定性实验排序，不生成工作流或自动执行。

交付：

- 只读取 e1 typed gaps 的 pure guidance projection。
- 固定 factor vector：boundary importance、hypothesis discrimination、gap closure、reproducibility、novelty、Target applicability、cost、mutation/version-drift risk。
- hard invalidity/prerequisite filter、稳定 Reason Code、逐 factor 报告和 deterministic tie-break。
- local Shadow result，可由 operator 忽略；不改变 Phase、Research、Memory 或工具状态。

测试：

- unknown/missing、缺前提、重复 action_fingerprint、跨 Snapshot、mutation risk 和稳定 tie-break。
- 受控案例验证首选项可关闭更多关键 gap，而不是只偏好低成本动作。
- Shadow 不创建 Experiment、不执行 Tool、不提升 Finding；无收益则删除或保持 Deferred。

Gate：完整 locked Cargo/Lean Gate；无 model judge、learned ranker、autonomous experiment generator、Workflow DSL 或事实写入。

### F1a：唯一配置驱动 Provider 装配路径（M4-S2 后置修复）

目标：在真实 Provider 测试和第一版 Chat 之前，消除通用运行入口中的 Provider 手工装配和端点/模型硬编码。

交付：

- 严格、显式版本化的 Provider Config v1，包含命名 Provider Profile、Capability Mapping 和 Agent Profile。
- 唯一 Provider Composition Root；`chat`、`doctor` 和所有 Live Smoke 只通过该路径取得相同配置语义。
- 凭据引用只保存专用 `CYBER_AGENT_*_API_KEY` 名称；真实值保持惰性解析，`doctor` 和 shell-only `chat` 不读取凭据。
- 当前 Responses binding 先保持单一实现；通用 Live 路径不出现 Provider 官方 URL或模型环境变量。

协议复用规则：

- Wire API 是显式配置，不根据 Provider/Model 名推断。
- 同一 Wire API 的请求编码、HTTP/SSE Transport 和响应解码只能有一个语义所有者。
- 第二个真实 Responses Provider 出现时先提取共享 Responses wire；仅 URL、Model、凭据引用或 Capability 不构成新 binding，不得复制 Adapter、Contract Suite 或 Live 装配。
- 新 Wire API 仅实现其可复用协议边界，不在 `chat`、`doctor` 或 Live Test 内写厂商特制流程。
- 不建设动态 Registry、通用 Router、自动 fallback 或插件系统；固定枚举只覆盖已经存在的 binding。

测试：

- 未知 Schema/字段、空值、悬空 Profile/Capability 引用和非法能力组合明确失败。
- Capability 只来自显式版本化 Mapping；Model 名不改变能力。
- Primary 与可选 Vision 通过同一 Composition Root 构造并保持身份隔离。
- Fake Credential Source 证明只解析配置声明的精确引用；静态检查和 `doctor` 不解析凭据。
- 协议专属离线 Contract/Golden Test 可直接构造 Adapter；通用 Live Smoke 必须使用 Composition Root。

禁止：

- 不实现 DeepSeek 或其他厂商 binding。
- 不运行真实 Live Test，不实现第一版 Chat AgentLoop。
- 不增加 Cargo 依赖、新事实源、Provider Router/Registry、Feature 或未来占位抽象。
- 不把端点、模型或 Key 写入 Event、Artifact、Snapshot、日志、Fixture 或仓库配置。

Gate：完整 locked Cargo/Lean Gate，加上通用运行路径 Provider URL/专用模型环境变量扫描，以及 Live Smoke 直接 Adapter 构造扫描。

### F1b：协议优先的共享 Responses HTTP/SSE Adapter

目标：让任意明确配置且符合已实现 Responses HTTP/SSE 契约的服务复用唯一 Adapter，不按厂商品牌复制代码。

交付：

- 提取唯一 `ResponsesHttpAdapter`，统一拥有 request encoding、HTTP/SSE transport 和 response decoding；删除厂商命名的生产 Adapter。
- Config v1 使用固定 `binding = "http_sse"` 与 `wire_api = "responses"`；Base URL、Model、`CYBER_AGENT_*_API_KEY` 引用和 Capability Mapping 都是显式数据，不产生厂商分支。
- `response.reasoning_text.delta` / `response.reasoning_text.done` 作为 fixture 证明的 Responses 扩展显式解码；Reasoning 与 assistant output 分离，未知事件不得静默丢弃。
- 无状态 Responses 服务通过 Capability Mapping 显式声明 `incremental_input = false`；AgentLoop 发送完整 History，不伪造 `previous_response_id`。不得根据 URL、模型名或凭据名推断该能力。
- `chat`、`doctor`、两个通用 Live Smoke 和所有已配置 Responses 服务复用同一 Composition Root 与同一 ModelProvider Contract Suite。

禁止：

- 不复制 Adapter、HTTP Client、SSE Loop、Request DTO、通用错误映射或 Contract Suite，也不为厂商名称增加薄 binding。
- 不把任何 Provider URL、Model 或 Key 写入通用 CLI/Live Test；只通过 Config v1 的 Base URL、Model 和精确凭据引用选择。
- 不实现 Chat Completions、第一版 Chat UI、自动 fallback、Router/Registry/Plugin 或 M4-S3。
- 未获得当前 Live Test 授权时只运行 loopback/fixture/Mock Gate。

Gate：完整 locked Cargo/Lean Gate；既有 Responses 回归不变；协议 fixture 覆盖 reasoning text、Tool Call、Usage、Completed 和未知事件；静态证明只有一个 Responses transport/decoder；如单独授权，再用明确配置执行一次固定合成 Live Smoke。

### F1c：第一版 Chat 接入现有 AgentLoop

目标：让 CLI `chat --config <path>` 通过同一 Composition Root 进入现有一个 AgentLoop，并完成第一版真实文本交互。

交付：

- `config -> AgentModelProfile -> ProviderComposition -> AgentLoop` 唯一路径。
- 复用已有 Project/Session、Prompt Profile、Context Planner、Tool Registry、RequestAudit 和 Full/Incremental 语义；不创建第二套轻量 Chat loop。
- 最小交互输入、Streaming 输出、Ctrl-C 取消和清晰终止；更完整行编辑/历史 UI 单独评估。
- `doctor`、Chat 和 Live Smoke 对同一配置给出一致 binding/capability 结果。

禁止：

- 不建设 TUI/Web、通用命令框架、第二 AgentLoop、模型 Router、团队/云服务或 M4-S3。
- 不以 Chat 为理由复制 Tool、History、Prompt、Context、Session 或 RequestAudit 所有者。

Gate：Mock/loopback 端到端 Chat 回归、项目隔离和审计回归、完整 locked Cargo/Lean Gate；真实 Chat 仅在用户单独授权 Live Test 时运行。

### F1d：本地配置内联 Provider API Key

目标：允许个人用户直接在显式 Provider Config v1 中保存 API Key，同时保留环境变量引用方式，不复制 Provider、Adapter 或 Chat 装配路径。

交付：

- 每个 Provider 必须且只能配置一个 `api_key` 或 `credential_ref`；既有 `credential_ref` 配置保持兼容。
- `api_key` 由唯一 Composition Root 绑定到该 Provider 的现有 CredentialSource 边界；Adapter、AgentLoop、CLI 和 Live Smoke 不增加内联凭据分支。
- `doctor` 可验证含内联 Key 的配置但不显示 Key；配置解析、校验、Provider 错误、Session、RequestAudit、Artifact 和 CLI 输出均不得复制 Key。
- 不强制权限、Keychain、加密 Vault 或环境变量中转；个人配置文件的保存位置和备份由用户负责。

禁止：

- 不允许同时配置 `api_key` 与 `credential_ref`，不允许缺少两者或使用空内联值。
- 不读取通用厂商变量，不回退到 Codex/ChatGPT 登录状态，不跨 Primary/Vision Provider 借用 Key。
- 不新增 Credential Manager、Secret Service、Provider Router/Registry、厂商 Adapter、Cargo 依赖、Live 请求或 M4-S3。

Gate：配置与 Composition 单测、CLI `doctor` 防泄漏、loopback Chat Authorization 与 Session/RequestAudit 非泄漏回归，以及完整 locked Cargo/Lean Gate。

### F1e：Responses Reasoning Continuation

目标：让需要客户端重放已完成 reasoning output item 的无状态 Responses 服务，通过现有 Config v1、唯一 Responses Adapter 和一个 AgentLoop 完成真实 Tool Loop。

交付：

- `ModelInputIR` v3 增加仅用于 Provider delivery 的 completed reasoning；Reasoning 不进入 Conversation/Session/Research/Memory 或用户输出。
- Responses Adapter 解码 completed `reasoning_text`，并按原始顺序编码 `reasoning -> function_call -> function_call_output`；无 Provider、Model、URL 或凭据分支。
- Full-only Capability Mapping 保留完整 provider-visible delivery；incremental 服务继续只发送严格 Delta。任何 Context 预算或状态路径无法保留 reasoning 时显式失败，不静默退回普通 History。
- RequestAudit 继续保存 full logical 与 actual delivered input；历史 ModelInputIR v2 可读，Replay 在请求匹配边界无损提升为 v3。
- 明文 reasoning 旁的冗余 opaque `encrypted_content` 保持 Adapter-local；encrypted-only 或未知 reasoning content 显式失败。

禁止：

- 不把 reasoning 伪装为 assistant text 或持久事实，不保存 opaque encrypted sidecar 或 Provider item ID。
- 不增加 DeepSeek/厂商 Adapter、Capability 推断、Router/fallback、配置字段、依赖、第二 History 或第二 AgentLoop。
- 不以 Live 兼容为理由放宽任意 Full RequestAudit mismatch；非 state-diff 路径只能接受移除 reasoning 后与逻辑输入完全一致的 replay。

Gate：Responses/AgentLoop/History/RequestAudit/Replay 专项与受影响测试、独立有限审计、完整 locked Cargo/Lean Gate；如单独授权，再用显式 `incremental_input=false` 的 Config v1 完成一个固定 DeepSeek `read_file` Live Tool Loop。

### M4-S4：真实长流程验收

目标：完成一个授权、可控的长流程漏洞研究任务。

模型组合验收：

- 文本 Primary + Vision Provider 完成图片分析、结构化 Observation 和后续主模型工具推理。
- 单一多模态 Primary 完成同一图片样例，并证明没有 Secondary 请求。
- `disabled` 和无可用图片 Provider 的负向路径返回稳定能力错误。
- Turn 边界切换 Agent Profile/VisionMode 后，旧 Primary/Vision Session 状态不被复用。
- 同一稳定人格下切换 DomainProfile/ResearchPhase，并证明 Prompt、Tool exposure、Session continuity 和历史审计不交叉或漂移。
- 每个 Sampling Step 的 Context/Tool/dispatch plan 一致；Deferred activation 不跨 Turn/Resume；ContextDeliveryMap 和 Evidence Spine 可重建。
- 两条 Live Provider 路径分别 opt-in；缺少任一凭据时继续完成另一条和全部 Mock/Fixture/Replay Gate。

必须量化：

- Model Requests 和 Token。
- Full/Incremental 命中与回退。
- Batch/并行收益。
- 重复读取和 Snapshot Hit。
- Memory Retrieval。
- 重复 Hypothesis/Experiment。
- acceptable first-action、invalid arguments/correction、unmatched tools、missed batch opportunities 和 Tool Schema token。
- Observation/Replay/Vision evidence-promotion boundary violations。
- Security retrieval recall/precision、重复读取/Token 变化，以及 guidance 的关键 gap closure/cost；Shadow 未证明收益不得晋升。
- Replay 回归覆盖。

没有证明收益的优化抽象必须删除或降级为 Deferred。

### M5-S1：CLI 充分性评估

仅当 CLI 在图片、Markdown 或长输出上有明确不足时，才实现 loopback-only Web feature。

不实现登录、多用户、远程部署或云会话。

### M5-S2：CI、Release 与个人分发

目标：可重复构建个人二进制。

交付：

- 精简 GitHub Actions。
- fmt/clippy/test/build --locked。
- macOS aarch64 release artifact 和 SHA-256。
- 不包含 API Key、项目 `.cyber-agent/` 或本地日志。
- 默认 `ci.yml` 不配置 API Secret，只运行 Mock/Fixture/Replay 测试。
- 独立 `live-smoke.yml` 只允许 `workflow_dispatch`，从 `live-test` Environment 向单个测试 step 注入 CI 专用 Key。
- Fork、普通 PR 和 `pull_request_target` 不得获得或触发 Live Test Secret。

## 7. 失败与阻塞处理

- 同一错误先定位最小失败层：Domain、Port、Adapter、Integration、Environment。
- 不通过增加新抽象绕开测试失败。
- 如果需要改变冻结架构，停止 Slice，创建 ADR 草案并报告。
- 如果需要外部凭据，只跳过被凭据阻塞的 live test，继续完成 Mock/Fixture/Replay 路径；不得尝试读取或修改当前 Codex 登录来解除阻塞。
- 如果工作树已有用户改动，保留并绕开；重叠无法安全处理时停止。
- 不删除未知文件，不重置工作树，不修改上游 Codex。

## 8. Codex Luna xhigh 任务模板

完整、可参数化并包含凭据/Git/Live Test 边界的主提示词见 `docs/development/LUNA_IMPLEMENTATION_PROMPT.md`。每个新开发会话优先使用该文件；以下内容仅保留为最短启动摘要：

```text
阅读仓库根目录 AGENTS.md、docs/README.md、docs/development/STATUS.md，
然后阅读 IMPLEMENTATION_PLAN.md 中 <SLICE_ID> 的完整章节、LEAN_CODE_GOVERNANCE.md 以及 task.md 相关架构和测试红线。

只实现 <SLICE_ID>，不要开始下一 Slice，不修改 codex/，不引入被禁止依赖。
先检查当前工作树和已有实现，避免重复或覆盖用户改动。
实现最小 Vertical Path，再补失败路径与测试。
运行该 Slice 要求的 fmt、clippy、test、build --locked、回归测试和 Lean Gate。
完成后更新 STATUS.md，列出改动、测试、Lean Review、剩余风险和下一 Slice，然后停止。
除非本次明确授权，不提交、不推送、不创建 PR 或 Release。
```

## 9. 计划变更规则

- 未完成 Slice 可以细分，但不能绕过原 Gate。
- 已完成 Slice 不追溯改写；修复作为独立 Fix Slice 或 ADR。
- 新依赖、新事实源、新服务、新 crate、新 Provider 和新外部协议都必须有 ADR。
- 实际 Eval 证明某优化无收益时，删除实现并在 ADR/STATUS 记录结果。
