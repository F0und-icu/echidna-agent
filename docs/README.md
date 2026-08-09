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
6. 最新的冻结架构链：`docs/architecture/CYBER_AGENT_ARCHITECTURE_V1.md` + `docs/architecture/CYBER_AGENT_ARCHITECTURE_V1_1.md`。
7. 已接受的 `docs/adr/ADR-*.md`。
8. `docs/development/GIT_VERSION_RELEASE.md` 和 `docs/development/DEVELOPMENT_ENVIRONMENT.md`。
9. `Codex架构与漏洞挖掘Agent复用评估.md`。
10. 只读上游参考 `codex/`。

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
| `docs/development/GIT_VERSION_RELEASE.md` | 分支、提交、PR、版本和发布规则 | 可以，通过 ADR 修改重大规则 |
| `docs/development/DEVELOPMENT_ENVIRONMENT.md` | 本地和 CI 环境、工具链、配置和诊断 | 可以，环境变化时更新 |
| `docs/development/LUNA_IMPLEMENTATION_PROMPT.md` | Luna 单 Slice 实现、验证和 Git 授权主提示词 | 可以，执行协议变化时更新 |
| `docs/development/UPSTREAM_PORT_LEDGER.md` | 从 Codex 移植代码的来源、License 和成本 | 每次 PORT 必须追加 |
| `docs/adr/ADR-0000-template.md` | 架构决策记录模板 | 模板本身少改 |
| `docs/adr/ADR-0002-capability-driven-primary-vision-models.md` | Primary + 可选 Vision Provider 决策 | ACCEPTED 后只由新 ADR 取代 |

## 4. Codex 执行入口

Codex Luna xhigh 或其他高推理 Codex 执行开发任务时，必须按顺序读取：

1. `AGENTS.md`
2. `docs/development/STATUS.md`
3. 当前 Slice 在 `docs/development/IMPLEMENTATION_PLAN.md` 中的完整章节
4. `task.md` 的相关架构章节和测试红线
5. 冻结架构 v1 基线、v1.1 增量及相关 ADR 中与该 Slice 相关的接口契约
6. 已有源码、测试和工作树状态

可直接复用的完整调用协议见 `docs/development/LUNA_IMPLEMENTATION_PROMPT.md`；它不能替代上述权威文件。

每次只实现 STATUS 指向的一个 Slice。完成验收后停止，不自动进入下一 Slice。

## 5. 状态与事实

- 架构事实：冻结架构 + 当前 task + ADR。
- 开发状态事实：`STATUS.md`。
- 代码事实：实际源码、测试和 `Cargo.lock`。
- Git 历史事实：提交、Tag 和 PR。
- 上游复用事实：`UPSTREAM_PORT_LEDGER.md`。
- 构建事实：本地命令输出和 GitHub Actions 结果。

聊天记录、模型记忆、计划工具状态和未提交的解释都不能替代这些本地文件。

## 6. 当前状态

当前已完成设计基线和 M0 的两个实现 Slice：

- M0-S1 单 Rust package、默认 CLI 二进制和离线 smoke tests 已完成。
- M0-S2 provider-neutral EventEnvelope、identity Upcaster、纯 Reducer 和最小 Event Ports 已完成。
- 已初始化本地 Git 并发布文档基线到私有 `main`。
- GitHub Actions 使用固定 Rust `1.93.1` 运行 fmt、clippy、test 和 build Gate。
- M0-S1/M0-S2 分别通过 PR `#1`/`#2` 交付；M1a-S1 为 `READY_NOT_AUTHORIZED`。
- `codex/` 仍是只读参考。

实现入口以 `docs/development/STATUS.md` 为准。
