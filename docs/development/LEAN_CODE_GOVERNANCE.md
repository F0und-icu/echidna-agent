# Cyber Agent 精简代码治理规范

## 0. 目的与适用范围

本规范用于防止 Cyber Agent 在持续开发中出现语义重复、公共接口膨胀、依赖堆叠、测试散落和无消费者抽象。

它约束所有本地或远程实现 Agent，包括在用户明确授权后参与开发的子 Agent。聊天摘要不能替代本文件；每个实现 Agent 都必须亲自阅读本文件、`AGENTS.md`、`STATUS.md` 和当前 Slice。

本规范是开发治理增量，不修改 Architecture v1 至 v1.6 的冻结语义。发生冲突时遵循 `docs/README.md` 的权威顺序。

## 1. 当前基线与迁移规则

- 项目继续保持一个 Rust package、一个默认二进制和一个 AgentLoop。
- Base Contract 只有一个 `Pragmatic Vulnerability Researcher` 身份；DomainProfile、ResearchPhase 和 AgentModelProfile 是三个正交轴，不产生组合角色或第二 Agent。
- 每个 model sampling Step 只有一个 immutable SamplingStepPlan；Tool catalog、ResolvedToolPlan 和 dispatch 不得保留第二份可变 exposure authority。
- `codex/` 继续作为固定提交的只读参考，不成为编译依赖。
- M1a-S2 已把 `src/lib.rs`、`src/domain/model.rs` 和 `src/domain/events.rs` 的永久就地测试迁移到顶层 `tests/`，并保留原行为覆盖。
- 后续每个 Slice 必须维持 `src/` inline-test 扫描通过；不得以“测试已通过”为由删除回归保护。
- 治理文档、已完成迁移和前一 Slice Gate 都不自动授权下一 Slice、Git 写入或 Live Test。

## 2. 语义唯一所有者

代码复用的目标不是减少每一个重复调用，而是保证同一业务语义只有一个实现所有者。

| 语义 | 唯一所有者 | 其他层允许的行为 |
|---|---|---|
| `ModelInputIR`、`ModelEvent`、`ModelCapabilities` | `domain/model` | Adapter 只做 wire 编解码并调用统一校验 |
| Provider-neutral Model Port 与通用 Provider 错误 | `ports/model` | Adapter 私有错误只在边界映射一次 |
| Streaming 完成、断流和应用级取消 | `app/model_streaming` 或后续唯一 AgentLoop 服务 | Transport 只提供流、释放和协议错误 |
| Primary/Vision 每 Turn 请求准入 | 私有 `app/model_request_budget` | Config 唯一路径验证额度；CLI 在初始 Vision 前创建并透传同一预算，显示 admission 而非 Provider usage；不新建 retry/计费/持久日志 |
| 可恢复停止与显式恢复 | App AgentLoop 的有限 typed classifier + 原 Session Resume | CLI 验证耐久/运行时 History 一致后提示；`/resume` 前禁止新任务，无自动 reasoning 丢弃或工具重试，未知和持久化错误仍 fatal |
| Responses 请求、响应和 SSE DTO | `adapters/responses` | 不得进入 Domain 或 App 契约；Provider 配置不得复制协议类型 |
| 最终文本核对 | 私有 `adapters/responses/response/text` | 单响应内按 wire 位置/身份核对；相同终值去重、合法前缀仅补后缀，冲突/错序拒绝，无自动续写或第二 History |
| CLI reasoning 展示与开发构建 wire debug | `adapters/responses/cli_output` | 既有 Composition Root 选择 CLI 呈现；不重建请求、不写第二份事实；release 保留 reasoning 但排除完整 debug |
| 终端内容转义与有界预览 | 私有 `terminal_content` | Tool/debug 仅复用派生视图；不修改事实、Tool 输出或 Context 预算 |
| Tool 执行展示 | 私有 `cli_chat/tool_output` | 使用已有 start/durable receipt 回调；长 ID 保留在原审计源；未知结果不得展示完成 |
| Vision 图像坐标系与范围 | `domain/vision/geometry` | 图片 Adapter 提供字节尺寸；Analysis、Observation 校验和 Replay 消费同一契约，不静默缩放/夹取区域 |
| 图片字节检查与 Artifact 解析 | `adapters/image` | CLI geometry 与 Responses 复用同一不可变 Blob/Artifact 解析；App 不解码图片或读路径 |
| Completed reasoning continuation | `domain/model` 的语义项 + AgentLoop provider delivery state | Conversation History 拒绝 reasoning；opaque encrypted sidecar 与 wire ID 保持 Adapter-local |
| Full 交付输入关系与增量 Tool 续接 | `domain/context/baseline` 的共享关系谓词和原 Baseline 比较器 | App 验证同 Turn 新增/旧 Tool 定义及完整前缀；RequestAudit 复用当前契约关系，不复制先前 ProviderSession；差量、预算和 provenance 仍由原 Context owner 产生 |
| Tool 类型、Effect 和执行收据 | `domain/tools` | Tool Adapter 只执行并返回结果 |
| Job/PTY 运行句柄身份 | 私有 `adapters/tools/process_identity` | 两个进程表使用独立 namespace 和 checked sequence；控制仅按原表精确查找，不解析旧序号重连 |
| 原始进程输出 | `adapters/tools/output` 捕获；`domain/tools/capture` 元数据/一致性；私有 `app/agent_loop/output_capture` 发布 | 复用 ArtifactStore 在 Receipt 前保存，原始 bytes 不缓存/入 History；read_output 复用本地 Blob 校验 owner，引用不升级 Research 证据 |
| 子命令检查点 | 固定 Tool Port 观察/确认；App 原始发布和 Session append；Domain 纯子项验证 | Adapter 不写 Event；先 Blob 和真实检查点再确认；子项不成为新 ToolCall/Receipt，Resume/Compaction 复用 History owner，UI 不阻塞管道 |
| Tool Call/Result 配对和 History 修复 | 一个纯 History Normalizer | AgentLoop、Context Planner、Adapter 不各自修复 |
| Prompt Fragment、排序和上下文预算 | 一个 Context Compiler/Planner | Provider Adapter 只序列化最终 IR |
| 稳定人格和 Base Contract | 一个版本化 Base Prompt 资源 | Personal/Project Prompt 不能替换证据纪律或运行时不变量 |
| DomainProfile | 一个固定 `source/web/mobile/binary` 目录与转换所有者 | 只提供领域 Prompt、TargetKind 和领域 Tool Exposure；不拥有阶段或模型选择 |
| ResearchPhase | 一个固定五阶段目录、Turn 边界选择和 Session 投影所有者 | 不拥有 Research 事实、Finding 状态或通用工作流 |
| Tool Exposure 合成 | 一个纯 Registry/Domain/Phase/Deferred-activation 合成所有者 | AgentLoop、Profile、Phase 和 `tool_search` 不各自复制优先级或 Hidden 规则 |
| Sampling Step 与 Tool dispatch binding | 一个 immutable SamplingStepPlan/ResolvedToolPlan owner | RequestAudit、Provider request 和 Tool executor 只消费同一 Plan；不查更新后的 raw catalog |
| Prompt renderer | ContextPlanner 内固定 `generic-v1` renderer | Prompt 资源和 Adapter 不复制 separator、render 或 provenance-map 规则 |
| Context provenance | ContextPlanner 产生的 ContextDeliveryMap | ContextSlot 决定位置；Origin/Authority/Retention/Freshness 不改顺序，Provider item ID 不进入 Domain |
| Tool guidance、Schema 与结构化错误 | `ToolSpec`/Tool contract 唯一所有者 | Adapter 只执行或在边界映射一次；Prompt 不复制参数规则 |
| 原始 Tool 输出和证据 | Event/Artifact 事实层 | Context/UI 可以各自生成有明确语义的派生视图 |
| Project/Research 事实 | Event/Artifact/Markdown 权威源 | Cache/Summary/Snapshot 只能删除重建 |
| Evidence relation | Research Ledger 的 typed EvidenceLink owner | Summary/Prompt/Replay/Eval/Vision 只能引用或投影，不能创建关系 |
| Retrieval 与 guidance | 固定版本化纯算法各一个 owner | Candidate pool 保留独立 eligibility；报告只进 Shadow/derived path |
| 交互 History/Receipt 检查点 | AgentLoop 固定阶段 + Session Writer/Reducer | Receipt 不确定性不被改写成虚假 Result；无自动执行重试 |
| 交互当前上下文装配 | CLI `context` 读取显式来源，AgentLoop 安装，ContextPlanner 编译 | 不增加目录发现、Prompt Registry 或另一套预算公式 |
| Compaction 候选与恢复 | CLI `compaction` 选择，原 Session/History owner 配对，原 ContextPlanner 预检 | 先严格验证历史前缀再重建当前引用；不得绕过 exact-TargetSnapshot owner |
| Session Receipt 投影缓存 | `research_projection` 的进程内派生 cache | 按精确来源复用；无独立持久事实源，不能靠 mtime 假定未变 |

重复调用统一所有者是允许的；复制其规则、错误判定、排序、截断或配对逻辑是不允许的。

## 3. 模块与抽象规则

1. 按变化原因和语义所有权拆模块，不按文件行数机械拆分。
2. 生产代码超过约 300 行，或同时因两个独立原因变化时触发拆分审查；这是审查信号，不是自动拆文件命令。
3. 第一次需求直接实现；第二次比较差异；第三个真实消费者出现后才提取通用抽象。
4. 唯一性、配对、规范化和失效等核心不变量从第一次实现起就必须只有一个所有者，不等待第三次重复。
5. 默认使用 `pub(crate)`；只有当前生产消费者实际需要的跨边界契约才能使用 `pub`。测试访问本身不是扩大公共 API 的理由。
6. 禁止没有清晰语义的 `common.rs`、`utils.rs`、`helpers.rs`、`base.rs` 或通用 `manager.rs` 杂物模块。
7. 禁止为未来 Slice 创建无消费者的 Trait、Feature、配置、事件、Schema、Prompt、TODO 框架或空适配层；仅测试消费者、无生产消费者的类型/API，以及未来占位，在 Slice 封存前必须删除或明确 Deferred。
8. 不建设通用 Hook Registry 或动态插件运行时。需要扩展时先使用 AgentLoop 内固定、命名且类型化的处理阶段；第二个真实实现出现后再评估 Trait。
9. 不因测试失败新增中间层或包装抽象；先定位 Domain、Port、Adapter、App 或 Environment 的最小失败层。
10. 不建设 Personality/Role、Prompt Stage、DomainProfile 或 ResearchPhase 动态 Registry。固定资源使用显式 enum/match 和一个组合路径；不要生成 `MobileVerifier` 一类笛卡尔积类型。
11. `ResearchPhase` 只改变当前目标、Prompt 和 Tool Exposure；它不能复制 Research Ledger 状态机，也不能从模型自由文本隐式切换。
12. 单个当前 renderer 不得产生 PromptDialect Trait/Registry/Config；第二实现必须先有真实消费者、固定 Eval 证据和 ADR。
13. 不把 Context provenance 填进每个 ContentItem，不把 Provider DeliveryItemId 上浮为 Domain/Research identity。
14. 不为检索建立 selector framework、默认向量/图数据库或远程裁判；固定算法先经 local Shadow 证明收益。
15. 经授权委派时，Terra xhigh 负责 Context、Session、Replay、Evidence、AgentLoop 及跨 Port/Adapter 边界；Luna xhigh 只负责单模块 Tool/Schema/Fixture/tests/docs 或小修。
16. 实现者不得审计自己的变更；Luna 的只读审计仅覆盖交接列明的有限文件和明确清单，跨边界最终审计由 Terra xhigh 或主 Agent 完成。审计必须限时，超时或无结论不得记为 `APPROVE`。
17. 每个 Slice 固定顺序必须精确为：专项测试 → 受影响测试 → 独立审计 → 修复 → 完整 Gate → Lean Review。审计必须限时；超时或无结论不得记为 `APPROVE`。

## 4. 公共 API、依赖与代码预算

每个 Slice 的 Scope Lock 必须声明：

- 新增和删除的生产模块。
- 新增的 `pub`/Port/Domain 契约及其真实消费者。
- 新增长期依赖的唯一职责、关闭的 default features、替代方案和编译成本。
- 新事件、配置、Prompt、Schema 和 Cache 的消费方、测试与移除条件。

强制规则：

- 同一职责只选择一套 HTTP、SSE、异步、图片、缓存或序列化技术栈。
- 新依赖必须符合当前 Slice，并按实施计划决定是否需要 ADR；不得顺手加入 SDK 或大依赖闭包。
- Feature flag 只有两个分支都真实存在且有测试时才能加入。
- 没有运行时路径、测试路径或明确近期消费者的代码必须在 Slice 完成前删除。
- 生产代码净增行数不是单独的拒绝理由，但无法解释所有权、消费者和删除条件时不得合并。

## 5. 测试布局与保留策略

### 5.1 生产源码规则

- `src/` 只保存生产代码，不保留就地 `#[cfg(test)] mod tests`、测试 Fixture 或测试专用实现。
- 临时就地测试允许在未提交的实现过程中用于探索，但 Slice Gate 前必须迁移或删除。
- 如果临时测试发现了缺陷或不变量，必须先把它转成 `tests/` 中的永久回归测试，之后才能删除临时版本。
- 不得删除永久回归测试来降低代码行数、减少维护工作或绕过失败。
- 正常/release 构建本来不会编译 `cfg(test)`，因此测试外置的目标是生产源码整洁和所有权清晰，不是伪造二进制体积收益。

### 5.2 顶层测试结构

```text
tests/
├── domain_<area>.rs
├── <port>_contract.rs
├── <vertical_path>_integration.rs
├── support/
│   └── mod.rs
└── fixtures/
    └── <protocol-or-domain>/
```

- 顶层 `tests/*.rs` 保存永久 Domain、Contract、Integration 和回归测试。
- `tests/support/` 只保存跨测试复用的构造器和断言，不复制生产逻辑。
- `tests/fixtures/` 保存稳定 wire/Event/History/Image 样本；一个事实样本只保存一份。
- 第二个 Adapter 出现时复用同一 Port Contract Suite；不得为每个 Adapter 复制一套契约断言。
- 测试优先通过所属模块的公开行为验证私有 helper；不得只为直接测试 helper 扩大生产 API。
- 跨 Port/Adapter 改动的受影响验证必须在独立审计前包含 `cargo test --locked --test domain_ports_contract`；不能等完整 Gate 才检查依赖方向，也不能为实现放宽既有 forbidden-dependency 测试。
- Live Test 继续 ignored/opt-in，和永久离线回归测试分离。

Gate 前运行：

```sh
if rg -n '#\[cfg\(test\)\]|^[[:space:]]*mod[[:space:]]+tests[[:space:]]*[;{]' src; then
  echo 'inline tests are forbidden under src/' >&2
  exit 1
fi
```

### 5.3 Provider 装配与 Live Test

- `chat`、`doctor` 和每个通用 Live Smoke 必须调用同一个配置解析与 Provider Composition Root；不得各自创建 Adapter。
- 通用运行路径不得出现 Provider 官方 URL、模型环境变量或隐式默认模型。协议专属离线 wire/contract fixture 可固定对应协议端点形状。
- 协议 Adapter Contract Test 可以直接构造 Adapter；该例外不能扩展到 Live Smoke、CLI 或 App 服务。
- Provider 配置必须且只能选择内联 `api_key` 或 `credential_ref`。引用式真实凭据只在显式 live `ModelProvider::stream` 路径惰性解析；内联值随配置解析进入进程，但 `doctor`、Mock、Fixture 和 Replay 不得发送或显示它。
- 相同 Wire API 只有一个 request encoding、HTTP/SSE transport 和 response decoding 所有者，并复用同一 ModelProvider Contract Suite。
- Provider URL、Model、凭据来源和 Capability Mapping 是配置数据，不构成厂商 binding。不得按品牌复制或包裹同一协议 Adapter；新代码必须由 fixture 证明真实 wire 差异。
- 固定 Binding/Wire 枚举只能列出已经实现的协议路径；不得以未来扩展为理由加入动态 Registry、Router、Factory 插件或未消费配置。
- Slice Lean Review 必须分别列出新增和删除的生产代码/装配点；协议泛化时必须删除被替代的厂商装配和命名。

### 5.4 Sampling、Prompt、Context 与 Tool 准确率

- Full Request 的固定 Prompt 顺序以 Architecture v1.2 为准，Step/renderer/provenance 边界以 v1.3 为准，ToolFailure/准确率可观测性边界以 v1.4 为准；Tool Specs 保持独立协议通道。Context Planner 仍是排序、预算、渲染和 Fingerprint 唯一所有者。
- SamplingStepPlan 在 Provider request 前一次捕获。RequestAudit、sent ToolSpecs、ToolCall validation/dispatch 和 Receipt 必须绑定同一 ResolvedToolPlan；不得在执行时回查更新后的 Registry exposure。
- `tool_search` activation 只在当前 Turn 后续 Steps 生效，下一 Turn/Resume 清空。当前 Step 及已产生 ToolCall 永不被新 activation 或 queued config 改写。
- `generic-v1` renderer identity 必须进入 fingerprint/audit/baseline/replay，并保持 ToolSpecs 在独立通道；没有第二真实 renderer 前禁止抽象化。
- M4-S3a1 只迁移最小 ContextSlot/Origin 且保持 bytes/顺序；完整 Authority/Retention/Freshness/ContextDeliveryMap 不得提前混入该 Slice。
- Base Contract 只有一个稳定人格。Personal Preferences 只影响用户偏好和表达，不得复制工具协议、证据边界或运行时约束。
- DomainProfile 和 ResearchPhase 使用固定编译期 Markdown 资源、稳定 ID 和 generation。历史 `prompt:mode:*` 审计内容只读保留；新语义不得通过批量重写历史来迁移。
- Phase 选择只在 Turn 边界生效。初始/legacy 默认和显式 operator 选择必须有稳定来源/Reason Code；模型自然语言不能改变 Phase。
- Tool Exposure 的 Hidden 终止、Domain/Phase override、Deferred activation 和 runtime availability 只能由一个纯函数/模块决定。它是上下文优化，不是权限层。
- 每个模型可见 Tool description 使用 `Use when`、`Do not use when`、`Prefer`、`Produces`；已知输入/输出不得用无约束的通用 object 逃避契约。
- 结构化 ToolFailure 仅允许 `pre_execution`、`execution` 两个阶段，并始终通过与原 ToolCall 配对的 ToolResult 返回。只有执行前参数失败不产生 Receipt，并允许模型发起新的纠正 ToolCall；任何已开始执行必须有真实 Receipt。Receipt/History/RequestAudit 等执行后持久化失败属于 App/AgentLoop 错误，不能改写真实成功 ToolResult 或伪造 Receipt；Runtime 不自动重试。
- 工具准确率 Eval 复用持久 Trace/Event/Receipt/Artifact，只产生 derived non-evidence 报告。只有持久 Trace 能诚实观测的指标才可报告；unmatched Replay termination 和 Observation/Replay/Vision promotion 在无 typed durable signal 前保持 Deferred，绝不假 0。Deferred activation 必须由同一 Turn 相邻 RequestAudit 的精确 ToolSpec transition 证明。固定 Fixture 不得通过新增 controlled public projection 入口绕过生产边界。
- 新增专用漏洞工具 wrapper 前，先用 `run_command(s)` 和至少三个重复真实消费者证明稳定协议；`http_request`、`inspect_environment` 和首个 AndroidManifest 外部 Extractor 按实施计划的独立 Slice 处理。

### 5.5 Evidence、Compaction、Retrieval 与 Guidance

- EvidenceLink 只由显式 Research update 创建；legacy upcaster 不猜测 control relation，Summary/Prompt/Model observation 不得提升证据。
- Session Summary v2 只保存 validated EvidenceSpine references 和 bounded lossy narrative，不保留可被当作权威的 copied verified-fact strings。
- ContextDeliveryMap 是 sidecar：覆盖最终 instruction spans 和 ContentItem positions，记录 Fragment/Subitem、hash 与 transform；不改变 model-visible bytes。
- Retrieval pipeline 固定为 exact identifiers → fielded BM25 → CJK/ASCII n-gram → RRF → authority/freshness tiers → stable tie-break。Candidate eligibility 不能被一个 global score 取代。
- Retrieval 与 evidence-gap/NextAction guidance 默认 Shadow、local、deterministic、explainable、non-evidence；未证明 recall/precision、重复读取、Token 或 gap closure 收益前不得进入 model-visible Context 或自动动作。
- Derived index/report 可删除重建；不得写正式 Research/Memory，不得调用 remote embedding/reranker/judge，不得引入 autonomous experiment/workflow。

## 6. Codex 借鉴和 PORT 准入

默认顺序：`REFERENCE -> REIMPLEMENT -> PORT SUBSET`。整 crate、整大模块或 Codex 产品层依赖不进入候选。

### 6.1 已定位候选

| 上游位置 | 本地决策 |
|---|---|
| `codex-rs/codex-api/src/endpoint/responses.rs` | REFERENCE；最小 HTTP Adapter 自行实现 |
| `codex-rs/codex-api/src/sse/responses.rs` | REIMPLEMENT 最小事件集；可借鉴完成/断流不变量 |
| `codex-rs/codex-api/tests/sse_end_to_end.rs` | 复用 Fixture 思路和边界案例，不复制产品测试框架 |
| `codex-rs/core/src/context_manager/normalize.rs` | 后续 Tool Call/Result 配对可 PORT SUBSET |
| `codex-rs/tools/src/response_history.rs` | 后续尾部保留与共享预算截断可 PORT SUBSET |
| `codex-rs/utils/image/src/lib.rs` 与对应测试 | 后续图片验证、尺寸预算和缩放可 PORT SUBSET |
| `codex-rs/core/src/image_preparation.rs` | REFERENCE；不移植 Feature/Analytics/Protocol 耦合 |
| `codex-rs/core/src/thread_rollout_truncation.rs` | REFERENCE；按本地 Context 类型重实现 |
| `codex-rs/core/src/session/step_context.rs`、`tools/router.rs`、`tools/spec_plan.rs` | REFERENCE；重实现更小 immutable SamplingStepPlan/ResolvedToolPlan，不复制产品 Router |
| `codex-rs/ext/skills/src/dynamic_skill_selector/` 与 `shadow_selection_experiment/` | REFERENCE；不移植 selector catalog/telemetry，以漏洞标识符和 Evidence authority 重实现固定 Shadow pipeline |

### 6.2 PORT 升级审查

出现任一条件时必须先写 ADR 或退回 REIMPLEMENT，不能直接复制：

- 预计超过约 200 行生产代码。
- 涉及超过两个上游实现文件，不含直接对应测试。
- 需要引入 Codex workspace crate、产品类型或新的依赖闭包。
- 无法为 Port 后代码指定唯一的本地所有者。
- 无法说明手工同步策略和删除/重写触发条件。

任何 PORT 必须先更新 `UPSTREAM_PORT_LEDGER.md` 的 PROPOSED 条目，记录固定上游提交、精确文件和测试、License/Notice、近似行数、替换类型、依赖、行为差异、本地测试、收益和移除条件。Gate 通过后才能标为 ACTIVE。

## 7. Slice Lean Gate

每个实现 Slice 在常规 fmt/clippy/test/build 之外必须回答并验证：

1. 本 Slice 新增了哪些语义？每个语义的唯一所有者在哪里？
2. 是否出现第二份 Capability、错误映射、Tool Pairing、截断、排序、Fingerprint 或取消实现？
3. 每个新 `pub`、依赖、Event、Config、Prompt、Schema 和 Cache 的消费者是谁？
4. 是否存在只为未来 Slice、只为测试访问或只为绕过失败而创建的抽象？
5. `src/` 是否没有就地测试，永久回归是否全部在 `tests/` 保留？
6. Codex 借鉴是否分类为 REFERENCE、REIMPLEMENT 或带 Ledger 的 PORT SUBSET？
7. 在本地 `codex/` 不存在的 CI 环境能否独立编译和测试？
8. 哪些实验代码、重复 Fixture、无消费者类型或无收益优化已在本 Slice 删除或 Deferred？
9. 是否仍只有一个人格，并且 DomainProfile、ResearchPhase、AgentModelProfile 没有被合并成角色组合或第二状态机？
10. Prompt/Phase/Tool Schema 变化是否由唯一所有者进入 RequestAudit、Fingerprint、Baseline invalidation 和 Replay Fixture？
11. Tool 描述、输入/输出 Schema 和结构化错误是否精确；是否有执行后自动重试、虚假 Receipt 或无法解释的 first-action/重复读取回归？
12. SamplingStepPlan 是否真正不可变，sent ToolSpecs、RequestAudit 与 dispatch 是否绑定同一 ResolvedToolPlan，Deferred activation 是否泄漏到下一 Turn/Resume？
13. Context provenance 是否通过一个 sidecar owner 完整覆盖且不污染 ContentItem/Provider identity；Summary/检索/Guidance 是否仍是 derived non-evidence？
14. Shadow 优化是否有固定 fixture、成本与 promotion/removal 条件；无收益实现是否删除或继续 Deferred？
15. 是否严格按专项测试 → 受影响测试 → 独立审计 → 修复 → 完整 Gate → Lean Review 的固定顺序完成；实现者与审计者是否不同？
16. Luna 审计是否只覆盖交接列明的有限文件和明确清单；跨 Port/Adapter 变更是否由 Terra 或主 Agent 完成最终审计？审计超时或无结论是否未记为 `APPROVE`？
17. 每个新增 `pub`/类型是否有当前生产消费者；仅测试消费者、无生产消费者或未来占位是否已删除或明确 Deferred？测试是否未新增 controlled public projection 入口？

STATUS 和 PR Body 必须记录 `Lean Review`：新增所有者、复用点、依赖/公共 API 变化、移除项以及未解决的治理债务。缺少该记录不得把 Slice 标为 COMPLETE。

## 8. 过渡 Gate 完成基线

M1a-S2 已完成以下一次性迁移；后续 Slice 将其视为必须持续保持的基线：

- 三个原就地测试模块已迁移或由已有公共行为测试覆盖。
- 原有回归行为不变量仍被覆盖；测试数量可以因合并表驱动用例变化，但不得降低覆盖。
- `src/` 的 inline-test 静态检查通过，并已加入默认 CI Workflow。
- 迁移没有扩大生产公共 API。
- M1a-S3 仍必须按 `STATUS.md` 获得独立授权后才能开始。
