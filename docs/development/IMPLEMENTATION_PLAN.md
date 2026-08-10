# Cyber Agent 可执行开发计划

## 0. 适用范围

本计划将 Architecture v1 基线和 v1.1 能力驱动模型组合增量转成可由 Codex Luna xhigh 或其他高推理 Codex 实现的 Vertical Slices。

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
6. 阅读冻结架构对应接口章节。
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

5. Handoff
   - 更新 STATUS：完成项、测试结果、Lean Review、剩余风险、下一 Slice。
   - 如果授权 Git 提交，按 Git 方案提交；否则保留工作树并报告。
   - 停止，不自动进入下一 Slice。

## 4. 全局 Definition of Done

一个 Slice 只有同时满足以下条件才完成：

- 目标行为存在可运行测试，不只存在类型或 TODO。
- `cargo fmt --all -- --check` 通过。
- `cargo clippy --all-targets --all-features -- -D warnings` 通过。
- `cargo test --all-targets --all-features` 通过；尚无 feature 时使用对应精简命令。
- `cargo build --locked` 通过；M0 创建 Cargo.lock 后始终使用 `--locked`。
- 没有修改 `codex/`。
- 没有直接依赖禁用 Codex crate。
- 没有未解释的新长期依赖。
- 每个语义只有一个所有者，没有复制 Capability、错误映射、Tool Pairing、截断、排序、Fingerprint 或取消逻辑。
- 新 `pub` 契约有真实跨边界消费者；测试访问不是扩大公共 API 的理由。
- 没有为未来 Slice、测试访问或绕过失败而创建的抽象、Feature、配置或 TODO 框架。
- 没有把 Cache/Summary/Replay 结果变成事实源。
- 新事件、Schema、配置和 Prompt 有消费者与测试。
- `src/` 不含就地 `#[cfg(test)] mod tests`，永久回归测试保留在顶层 `tests/`。
- 无消费者代码、重复 Fixture 和无收益优化已删除或明确 Deferred。
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
- OpenAI live 测试默认 ignored，只有 `CYBER_AGENT_LIVE_TESTS=1` 和 `CYBER_AGENT_OPENAI_API_KEY` 同时存在时才能开启。
- 默认测试不能依赖公网、真实设备或用户 API Key。
- Live 测试只使用固定合成输入，不读取真实项目 Memory、Session、Artifact 或漏洞目标。
- 凭据 Contract Test 使用 Fake Credential Source，证明不会读取 `OPENAI_API_KEY`、`CODEX_HOME`、`~/.codex` 或 Codex OAuth/session data。

### 5.4 Golden/Fixture

- Event 每个历史 Schema 至少一个 fixture。
- ModelInputIR 与 OpenAI 编码使用 snapshot fixture。
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
- Mode Pack 和 Prompt/Model Profile。
- AgentModelProfile：一个 Primary、可选 Vision 和 VisionMode。
- `auto`、`primary_direct`、`secondary_tool`、`disabled` 的纯 Vision Route Resolver。
- `doctor` 的 Provider/Mode Capability 组合检查。

测试：

- Deferred Tool 载入导致 Baseline Reset。
- Schema Token 指标。
- 不同 Mode 的 Prompt/Tool 指纹稳定。
- Capability 矩阵穷举、无模型名猜测、无价格/延迟/失败驱动的隐式 fallback。
- Profile/VisionMode 只在 Turn 边界切换，必要时重置 Primary Baseline。

### M3-S3：Compaction、Resume 与 Request Audit

目标：长会话可恢复并能重建语义请求。

交付：

- History 归一化。
- Session Summary 和 Compaction。
- RequestAuditManifest 和内容 Blob。
- `/prompt inspect`、`/snapshot`、`/metrics`。

测试：

- Tool Pair 不被拆散。
- Summary 不自动清空活跃 Baseline。
- Resume 首次 Full Request。
- 源 Prompt/Memory 改动后仍能重建历史语义请求。

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

### M4-S3：Observation 与 TargetDelta

目标：外部工具返回结构化观察并比较版本攻击面。

交付：

- 第一批 Observation Kind。
- 至少一个外部 Extractor 示例。
- 两个 TargetSnapshot 的 TargetDelta。

Gate：差分每项都能回到 Tool Call、Artifact 和 Snapshot。

### M4-S4：真实长流程验收

目标：完成一个授权、可控的长流程漏洞研究任务。

模型组合验收：

- 文本 Primary + Vision Provider 完成图片分析、结构化 Observation 和后续主模型工具推理。
- 单一多模态 Primary 完成同一图片样例，并证明没有 Secondary 请求。
- `disabled` 和无可用图片 Provider 的负向路径返回稳定能力错误。
- Turn 边界切换 Agent Profile/VisionMode 后，旧 Primary/Vision Session 状态不被复用。
- 两条 Live Provider 路径分别 opt-in；缺少任一凭据时继续完成另一条和全部 Mock/Fixture/Replay Gate。

必须量化：

- Model Requests 和 Token。
- Full/Incremental 命中与回退。
- Batch/并行收益。
- 重复读取和 Snapshot Hit。
- Memory Retrieval。
- 重复 Hypothesis/Experiment。
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
