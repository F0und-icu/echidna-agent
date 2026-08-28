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
7. 最新的冻结架构链：`docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` + `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md`。
8. 已接受的 `docs/adr/ADR-*.md`。
9. `docs/development/GIT_VERSION_RELEASE.md` 和 `docs/development/DEVELOPMENT_ENVIRONMENT.md`。
10. `Codex架构与漏洞挖掘Agent复用评估.md`。
11. 只读上游参考 `codex/`。

冻结架构是某一时点的不可变设计基线，但 `task.md` 可以通过新的 ADR 和新架构版本继续演进。不得直接修改已封存版本来掩盖设计变化。

## 3. 文档地图

| 文件 | 用途 | 是否允许日常修改 |
|---|---|---|
| `task.md` | 当前产品范围、架构和总里程碑 | 可以，必须有理由 |
| `AGENTS.md` | Codex 的持久开发约束、验证命令和停止条件 | 可以，需独立文档提交 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` | 架构 v1 冻结基线 | 封存后不修改 |
| `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md` | 架构 v1.1 能力驱动模型组合增量 | 封存后不修改 |
| `docs/architecture/SEALED_MANIFEST.md` | 封存来源、提交基线和 Hash 清单 | 只追加新封存记录 |
| `docs/development/IMPLEMENTATION_PLAN.md` | 按 Slice 可执行的开发步骤和验收门 | 可以，不能追溯改写已完成 Slice |
| `docs/development/STATUS.md` | 当前唯一工作状态和下一动作 | 每个 Slice 开始/结束更新 |
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

## 4. Codex 执行入口

Codex Luna xhigh 或其他高推理 Codex 执行开发任务时，必须按顺序读取：

1. `AGENTS.md`
2. `docs/development/STATUS.md`
3. 当前 Slice 在 `docs/development/IMPLEMENTATION_PLAN.md` 中的完整章节
4. `docs/development/LEAN_CODE_GOVERNANCE.md`
5. `task.md` 的相关架构章节和测试红线
6. 冻结架构 v1 基线、v1.1 增量及相关 ADR 中与该 Slice 相关的接口契约
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

M0 至 M4-S2 以及 F1a/F1b/F1c/F1d 集成交付均已远端封存。当前已具备 Config v1、唯一 Provider Composition Root、协议优先 Responses HTTP/SSE Adapter、第一版 AgentLoop Chat，以及内联 `api_key`/引用式 `credential_ref` 二选一配置，并通过一次合成 DeepSeek Live Chat。

M4-S3 Observation/TargetDelta 没有开始，`codex/` 仍是固定提交的只读参考。

实现入口以 `docs/development/STATUS.md` 为准。
