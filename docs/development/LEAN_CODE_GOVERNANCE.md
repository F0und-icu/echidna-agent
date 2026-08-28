# Cyber Agent 精简代码治理规范

## 0. 目的与适用范围

本规范用于防止 Cyber Agent 在持续开发中出现语义重复、公共接口膨胀、依赖堆叠、测试散落和无消费者抽象。

它约束所有本地或远程实现 Agent，包括在用户明确授权后参与开发的子 Agent。聊天摘要不能替代本文件；每个实现 Agent 都必须亲自阅读本文件、`AGENTS.md`、`STATUS.md` 和当前 Slice。

本规范是开发治理增量，不修改 Architecture v1/v1.1 的冻结语义。发生冲突时遵循 `docs/README.md` 的权威顺序。

## 1. 当前基线与迁移规则

- 项目继续保持一个 Rust package、一个默认二进制和一个 AgentLoop。
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
| Responses 请求、响应和 SSE DTO | `adapters/responses` | 不得进入 Domain 或 App 契约；Provider 配置不得复制协议类型 |
| Tool 类型、Effect 和执行收据 | `domain/tools` | Tool Adapter 只执行并返回结果 |
| Tool Call/Result 配对和 History 修复 | 一个纯 History Normalizer | AgentLoop、Context Planner、Adapter 不各自修复 |
| Prompt Fragment、排序和上下文预算 | 一个 Context Compiler/Planner | Provider Adapter 只序列化最终 IR |
| 原始 Tool 输出和证据 | Event/Artifact 事实层 | Context/UI 可以各自生成有明确语义的派生视图 |
| Project/Research 事实 | Event/Artifact/Markdown 权威源 | Cache/Summary/Snapshot 只能删除重建 |

重复调用统一所有者是允许的；复制其规则、错误判定、排序、截断或配对逻辑是不允许的。

## 3. 模块与抽象规则

1. 按变化原因和语义所有权拆模块，不按文件行数机械拆分。
2. 生产代码超过约 300 行，或同时因两个独立原因变化时触发拆分审查；这是审查信号，不是自动拆文件命令。
3. 第一次需求直接实现；第二次比较差异；第三个真实消费者出现后才提取通用抽象。
4. 唯一性、配对、规范化和失效等核心不变量从第一次实现起就必须只有一个所有者，不等待第三次重复。
5. 默认使用 `pub(crate)`；只有真实跨边界契约才能使用 `pub`。测试访问本身不是扩大公共 API 的理由。
6. 禁止没有清晰语义的 `common.rs`、`utils.rs`、`helpers.rs`、`base.rs` 或通用 `manager.rs` 杂物模块。
7. 禁止为未来 Slice 创建无消费者的 Trait、Feature、配置、事件、Schema、Prompt、TODO 框架或空适配层。
8. 不建设通用 Hook Registry 或动态插件运行时。需要扩展时先使用 AgentLoop 内固定、命名且类型化的处理阶段；第二个真实实现出现后再评估 Trait。
9. 不因测试失败新增中间层或包装抽象；先定位 Domain、Port、Adapter、App 或 Environment 的最小失败层。

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

STATUS 和 PR Body 必须记录 `Lean Review`：新增所有者、复用点、依赖/公共 API 变化、移除项以及未解决的治理债务。缺少该记录不得把 Slice 标为 COMPLETE。

## 8. 过渡 Gate 完成基线

M1a-S2 已完成以下一次性迁移；后续 Slice 将其视为必须持续保持的基线：

- 三个原就地测试模块已迁移或由已有公共行为测试覆盖。
- 原有回归行为不变量仍被覆盖；测试数量可以因合并表驱动用例变化，但不得降低覆盖。
- `src/` 的 inline-test 静态检查通过，并已加入默认 CI Workflow。
- 迁移没有扩大生产公共 API。
- M1a-S3 仍必须按 `STATUS.md` 获得独立授权后才能开始。
