# Cyber Agent 文档索引与权威顺序

## 1. 文档目的

本目录保存 Cyber Agent 的冻结架构、开发实施计划、Git/版本/发布规则、开发环境规则和长期决策记录。

目标是让新的 Codex 开发会话在没有历史聊天上下文的情况下，也能从本地文件恢复项目状态，并按一个可验证的 Vertical Slice 开发。

## 2. 权威顺序

发生冲突时按以下顺序处理：

1. 当前用户明确指令。
2. 仓库根目录 `AGENTS.md`。
3. `docs/development/STATUS.md` 中当前 Slice 和阻塞状态。
4. `task.md` 中当前产品范围、架构约束和里程碑。
5. `docs/development/IMPLEMENTATION_PLAN.md`。
6. `docs/development/LEAN_CODE_GOVERNANCE.md`。
7. 最新的冻结架构链：`docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` 至 `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_20.md`；ADR-0030 约束当前事实与回答指导的有限范围，ADR-0029 约束最终文本与精确激活到期，ADR-0028/0027 约束增量激活与显式哈希新鲜度，ADR-0026 约束终端投影，ADR-0025 约束环境指导与旧 Prompt 选择兼容，ADR-0024/0023/0022/0021 保留真实子项检查点、原始输出、运行句柄和有界恢复。
8. 已接受的 `docs/adr/ADR-*.md`。
9. `docs/development/GIT_VERSION_RELEASE.md` 和 `docs/development/DEVELOPMENT_ENVIRONMENT.md`。
10. `Codex架构与漏洞挖掘Agent复用评估.md`。
11. 只读上游参考 `codex/`。

冻结架构是某一时点的不可变设计基线，但 `task.md` 可以通过新的 ADR 和新架构版本继续演进。不得直接修改已封存版本来掩盖设计变化。

当前读取完整冻结链至v1.20，相关ADR见上述权威顺序。R6-04整理本地回归基线与状态入口；R6-SC1复用既有输入Schema定义，二者不增加架构版本。R6-SC1实验和测量见 [Schema复用报告](development/R6_SC1_SCHEMA_REUSE_MEASUREMENT.md)。准确状态和下一动作见STATUS；历史状态存档不重新授权Live或交付，远端hash封存仍未授权。

随后单独授权的 [R6-SC1 Live](development/R6_SC1_LIVE_2026_09_20.md) 已完成真实Schema调用和重启回读核验，并获独立事后结果复核 RESULT APPROVE（不是事前脚本审查）；措辞/覆盖限制和绝对usage如实保留，不推定token收益。已审查代码交付私有Draft PR #84，未合并、未跑Actions。

## 3. 文档地图

| 文件 | 用途 | 是否允许日常修改 |
|---|---|---|
| `task.md` | 当前产品范围、架构和总里程碑 | 可以，必须有理由 |
| `AGENTS.md` | Codex 的持久开发约束、验证命令和停止条件 | 可以，需独立文档提交 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` | 架构 v1 冻结基线 | 封存后不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md` | 架构 v1.1 能力驱动模型组合增量 | 封存后不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_2.md` | 架构 v1.2 稳定研究人格、DomainProfile、ResearchPhase 和工具准确率增量 | 本地冻结；远端封存前不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_3.md` | 架构 v1.3 不可变 Sampling Step、Context 来源、证据压缩与确定性研究引导增量 | 本地冻结；远端封存前不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_4.md` | 架构 v1.4 ToolFailure 执行边界与持久 Trace 准确率可观测性增量 | 本地冻结；远端封存未授权前不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_5.md` | 架构 v1.5 Provider reasoning continuation 与双 History 权威边界 | 本地冻结；远端封存未授权前不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_6.md` | 架构 v1.6 冗余 encrypted reasoning sidecar 的 Adapter-local 边界 | 本地冻结；远端封存未授权前不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_7.md` | 架构 v1.7 交互连续性、当前上下文与预算恢复；ADR-0017 | 本地冻结；远端封存待授权 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_8.md` | 架构 v1.8 默认 CLI 思考展示、仅开发构建 debug；ADR-0018 | 本地冻结；远端封存待授权 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_9.md` | 架构 v1.9 可读工具/调试内容与大窗口预算；ADR-0019 | 本地冻结；远端封存未授权 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_10.md` | 架构 v1.10 Vision 送入图坐标系与历史未知语义；ADR-0020 | 本地冻结；验证见 STATUS，远端封存未授权 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_11.md` | 架构 v1.11 共享请求准入与显式恢复；ADR-0021 | 本地冻结；验证见 STATUS，远端 hash 封存未授权 |
| `docs/architecture/SEALED_MANIFEST.md` | 封存来源、提交基线和 Hash 清单 | 只追加新封存记录 |
| `docs/development/IMPLEMENTATION_PLAN.md` | 按 Slice 可执行的开发步骤和验收门 | 可以，不能追溯改写已完成 Slice |
| `docs/development/STATUS.md` | 当前唯一工作状态和下一动作 | 每个 Slice 开始/结束更新 |
| `docs/development/STATUS_HISTORY_THROUGH_2026_09_19.md` | 截至2026-09-19的完整状态历史，原字节保留 | 仅历史证据，不作为当前执行入口 |
| `docs/development/LEAN_CODE_GOVERNANCE.md` | 语义所有权、测试外置、抽象/依赖预算、Codex PORT 和删除 Gate | 可以，治理规则变化时更新 |
| `docs/development/GIT_VERSION_RELEASE.md` | 分支、提交、PR、版本和发布规则 | 可以，通过 ADR 修改重大规则 |
| `docs/development/DEVELOPMENT_ENVIRONMENT.md` | 本地和 CI 环境、工具链、配置和诊断 | 可以，环境变化时更新 |
| `docs/development/LUNA_IMPLEMENTATION_PROMPT.md` | Luna 单 Slice 实现、验证和 Git 授权主提示词 | 可以，执行协议变化时更新 |
| `docs/development/UPSTREAM_PORT_LEDGER.md` | 从 Codex 移植代码的来源、License 和成本 | 每次 PORT 必须追加 |
| `docs/adr/ADR-0000-template.md` | 架构决策记录模板 | 模板本身少改 |
| `docs/adr/ADR-0002-capability-driven-primary-vision-models.md` | Primary + 可选 Vision Provider 决策 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0003-minimal-openai-http-sse-stack.md` | M1a-S2 最小 HTTP/SSE 依赖与取消边界 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0006-single-provider-composition-root.md` | F1a 唯一配置驱动 Provider 装配和协议复用边界 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0007-protocol-first-responses-adapter.md` | F1b 协议优先 Responses Adapter 与复用规则 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0008-local-inline-provider-credentials.md` | F1d 本地内联或引用式 Provider 凭据二选一 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0009-stable-research-persona-domain-profile-and-phase.md` | 单一稳定人格、领域与研究阶段正交组合及工具准确率决策 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0010-immutable-sampling-step-and-tool-plan.md` | 不可变 SamplingStepPlan、ResolvedToolPlan 与 Turn-local Deferred 生命周期 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0011-context-provenance-evidence-relations-and-compaction.md` | ContextDeliveryMap、EvidenceLink 与 Evidence-Spine Compaction | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0012-deterministic-security-retrieval-and-evidence-gap-guidance.md` | 本地确定性安全检索与证据缺口/实验引导 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0013-versioned-prompt-renderer-boundary.md` | 单一 `generic-v1` Prompt renderer 版本边界 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0014-narrow-tool-failure-and-accuracy-observability.md` | M4-S3b ToolFailure 边界、持久 Trace 准确率和 Deferred activation 审计证明 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0015-responses-reasoning-continuation.md` | F1e completed reasoning 的 Provider delivery 与 History 隔离 | ACCEPTED 后只由新 ADR 取代 |
| `docs/adr/ADR-0016-redundant-encrypted-reasoning-sidecar.md` | 明文 reasoning 旁冗余 encrypted sidecar 保持 Adapter-local | ACCEPTED 后只由新 ADR 取代 |

## 4. Codex 执行入口

Codex Luna xhigh 或其他高推理 Codex 执行开发任务时，必须按顺序读取：

1. `AGENTS.md`
2. `docs/development/STATUS.md`
3. 当前 Slice 在 `docs/development/IMPLEMENTATION_PLAN.md` 中的完整章节
4. `docs/development/LEAN_CODE_GOVERNANCE.md`
5. `task.md` 的相关架构章节和测试红线
6. 冻结架构 v1 至当前最新版本的完整 additive chain 及相关 ADR 中与该 Slice 相关的接口契约
7. 已有源码、测试和工作树状态

用户明确授权开发子 Agent 时，每个子 Agent 也必须自行读取上述文件；父 Agent 的摘要不能替代本地治理文件。

可直接复用的完整调用协议见 `docs/development/LUNA_IMPLEMENTATION_PROMPT.md`；它不能替代上述权威文件。

每次只实现 STATUS 指向的一个 Slice。完成验收后停止，不自动进入下一 Slice。

## 5. 状态与事实

- 架构事实：冻结架构 + 当前 task + ADR。
- 开发状态事实：`STATUS.md`。
- 代码事实：实际源码、测试和 `Cargo.lock`。
- Git 历史事实：提交、Tag 和 PR。
- 上游复用事实：`UPSTREAM_PORT_LEDGER.md`。
- 代码精简与测试布局事实：`LEAN_CODE_GOVERNANCE.md`。
- 构建事实：本地命令输出和 GitHub Actions 结果。

聊天记录、模型记忆、计划工具状态和未提交的解释都不能替代这些本地文件。

## 6. 当前状态

M0 至 M5-S2 以及 F1a-F1e/F2a 已远端封存。当前 F3 修复交互式运行连续性，具体审计和 Gate 进展以 STATUS 为准；本轮不增加非交互执行或 benchmark 适配，不触发 Git/Actions/Live。

Architecture v1.5/v1.6 已在本地冻结：completed reasoning 仅属于 Provider delivery，明文存在时冗余 opaque encrypted sidecar 不进入 Core；encrypted-only 与未知 content 继续显式失败。

Architecture v1.7 / ADR-0017 是 F3 的本地增量约束；历史封存和 Live 结果不会被当作 F3 的验证。`codex/` 仍是固定提交的只读参考。

实现入口以 `docs/development/STATUS.md` 为准。
