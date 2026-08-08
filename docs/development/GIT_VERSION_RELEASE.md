# Git、版本与发布管理方案

## 0. 当前边界

- GitHub 私有仓库：`F0und-icu/cyber-agent`。
- 文档基线已发布到远程 `main`；后续实现仍按 Slice 独立提交。
- 本地根目录已初始化 `.git`，当前分支为 `main`。
- `codex/` 是独立的只读上游 Git 仓库，绝不能加入 Cyber Agent 历史。
- 本文件是方案，不授权当前会话初始化、提交或推送。

## 1. Git 初始化方案

首次文档基线已按用户明确授权完成；后续 Git 操作仍需对应用户授权。

推荐顺序：

1. 再次确认当前根目录不是 Git 仓库。
2. 确认 `codex/` 上游工作树干净且提交与封存基线一致。
3. 创建 `.gitignore`，第一条必须忽略 `/codex/`。
4. 同时忽略构建输出、项目运行状态、密钥和本地编辑器文件。
5. `git init -b main`。
6. 只暂存 Cyber Agent 文档和后续自己的源码。
7. 检查 staged 列表中不存在 `codex/`、`.cyber-agent/` 研究数据、API Key 或本地日志。
8. 创建文档基线初始提交。
9. 添加私有远程，但只有用户明确授权后才 push。

初始 `.gitignore` 至少包括：

```gitignore
/codex/
/target/
**/.cyber-agent/
.env
.env.*
!.env.example
*.pem
*.key
.DS_Store
.idea/
.vscode/
```

项目未来会研究真实漏洞，`.cyber-agent/` 默认属于每个目标项目的本地研究状态，不能进入 Cyber Agent 源码仓库。

## 2. Trunk-based 分支模型

个人维护不使用长期 develop/release 分支。

- `main`：始终可编译、可测试。
- 每个 Vertical Slice 一个短分支。
- 修复从当前 main 创建短分支。
- Release 由 Tag 标识，不保留长期 release branch。

分支命名：

```text
feat/m0-s1-bootstrap
feat/m1a-s2-openai-streaming
feat/m2-s3-artifact-lineage
fix/m1b-baseline-reset
refactor/context-planner-cache
docs/architecture-v1-1
chore/dependency-update
```

如果由 Codex 创建分支，可加 `codex/` 前缀，但 Slice ID 必须保留：

```text
codex/feat-m1a-s3-tool-loop
```

## 3. 提交规则

使用 Conventional Commits 的轻量子集：

```text
feat(core): add model input IR
feat(tools): add resource-key scheduler
fix(provider): reset baseline on tool schema change
test(events): cover corrupted tail recovery
docs(architecture): record target snapshot decision
refactor(context): isolate fragment renderer
chore(deps): update serde within lockfile
```

规则：

- 一次提交只表达一个可解释意图。
- 代码和直接对应测试尽量同提交。
- 纯机械格式化不能夹带语义改动。
- 不提交失败构建、临时调试打印、真实目标数据或请求日志。
- 不使用 `git add .` 代替 staged 内容检查。
- 不 amend、rebase 或 force-push 已共享分支，除非用户明确要求。
- 不自动提交；当前任务必须明确授权 Git 写操作。

推荐提交粒度：

- Contract/type + unit tests。
- Adapter + contract tests。
- Integration path + fixtures。
- Documentation/status update。

如果一个 Slice 很小，可以合并为一个提交；如果超过可审查范围，按上述边界拆分。

## 4. Pull Request 规则

初始文档基线可以在用户授权后直接建立 main。此后建议每个 Slice 使用 Draft PR：

PR 标题：

```text
M1b-S2: add incremental Responses delivery and fallback
```

PR Body 必须包含：

- Slice ID 和目标。
- 架构/ADR 引用。
- 实际改动。
- 未实现内容。
- 测试命令和结果。
- Schema/配置兼容性。
- 新依赖与理由。
- 从 Codex PORT 的来源记录。
- 风险与回滚方法。

合并条件：

- Slice Gate 全部通过。
- GitHub Actions 必需 Job 通过。
- STATUS 已更新。
- 没有未解决的架构冲突。
- 没有 `codex/`、密钥、目标数据或本地 Artifact 被加入。

个人仓库推荐 Squash Merge，使一个 Slice 在 main 上对应一个可回滚提交；重要的内部提交信息写入 PR Body。

## 5. 版本体系

### 5.1 软件版本

使用 SemVer。达到第一个稳定真实研究验收前保持 `0.y.z`。

建议里程碑：

| 软件版本 | Gate |
|---|---|
| `0.0.1` | M0 骨架、工具链和 CI 基础通过 |
| `0.1.0` | M1a 全量请求和单工具闭环 |
| `0.2.0` | M1b 增量请求和回退 |
| `0.3.0` | M1c Batch、Job、Resource Key、PTY |
| `0.4.0` | M2 项目、事件、研究、记忆、Artifact |
| `0.5.0` | M3 Context、Compaction、Mode、多模态 |
| `0.6.0` | M4 Replay、Observation、真实 Eval |
| `1.0.0` | 至少一个真实长流程验收，Schema 演进和恢复承诺稳定 |

开发中可以发布：

```text
0.2.0-alpha.1
0.2.0-beta.1
0.2.0-rc.1
```

### 5.2 独立协议版本

以下版本不与软件 SemVer 强绑定：

- Event `schema_family/schema_version`
- ModelInputIR schema version
- Tool Spec/Observation schema version
- Project config schema version
- Cache schema version
- Frozen Architecture version

规则：

- 软件小版本可以支持多个历史 Schema。
- Schema 升级必须有 Upcaster 和 fixture。
- Cache Schema 不兼容可以删除重建。
- Event/Research/Artifact 权威数据不能通过删除解决兼容问题。

### 5.3 架构版本

- `Architecture 1.0.0` 对应当前封存文件。
- 兼容澄清发布 `1.0.x`。
- 新增可选能力且不改变核心事实源发布 `1.x.0`。
- 改变 Event/Tool/Model/Artifact 核心契约发布下一主版本。

架构文件一旦封存不修改，使用新文件和 SEALED_MANIFEST 记录。

## 6. Changelog 和 ADR

开始代码后创建 `CHANGELOG.md`，遵循 Keep a Changelog 的结构：

- Added
- Changed
- Fixed
- Removed
- Migration

`Unreleased` 只记录对用户或开发者可观察的变化，不堆积内部提交列表。

以下变化必须有 ADR：

- 新事实源或数据库。
- 新 crate/workspace。
- 新 Provider 或协议。
- Event/Tool/Artifact/ModelInputIR 不兼容变化。
- 新长期服务或运行进程。
- 修改 FullAccess、项目隔离或日志边界。
- 修改上游 Codex 复用策略。

## 7. Tag 与 Release

使用 annotated tag：

```text
v0.1.0
v0.2.0-rc.1
v1.0.0
```

如果本机已配置 Git SSH/GPG signing，推荐签名 Tag；未配置时不为了发布引入新的密钥流程。

Release Gate：

- main 干净且 CI 通过。
- Cargo.lock 已提交。
- `cargo build --locked --release --target aarch64-apple-darwin` 通过。
- Release binary 生成 SHA-256。
- `cyber-agent --version` 与 Tag 一致。
- CHANGELOG 已从 Unreleased 移入版本节。
- Migration Notes 完整。
- Artifact 中不存在 API Key、`.cyber-agent/`、目标文件或本地日志。

## 8. GitHub Actions 设计

### 8.1 Pull Request CI

触发：`pull_request` 和 main push。

Jobs：

1. quality
   - checkout
   - 安装 `rust-toolchain.toml` 固定工具链
   - `cargo fmt --all -- --check`
   - `cargo clippy --locked --all-targets --all-features -- -D warnings`
   - `cargo test --locked --all-targets --all-features`

2. macos-build
   - macOS runner
   - 添加 `aarch64-apple-darwin` target
   - `cargo build --locked --release --target aarch64-apple-darwin`
   - 生成 SHA-256 供检查，不在普通 PR 发布

3. architecture-guard
   - 检查只有一个 Cargo package。
   - 检查禁止的 Codex crate 不在 `cargo metadata`。
   - 检查 `codex/` 不被 Git 跟踪。
   - 检查封存架构文件没有被修改。

Actions 必须固定到明确版本；条件允许时固定 commit SHA。升级 Action 作为独立 chore PR。

### 8.2 Release Workflow

触发：匹配 `v*` 的 Tag，且 Tag 所指提交在 main 历史中。

步骤：

- 重跑 Quality。
- 构建 macOS aarch64 release。
- 生成 `cyber-agent-<version>-aarch64-apple-darwin.tar.gz`。
- 生成 `SHA256SUMS`。
- 上传 GitHub Release Artifact。

Workflow 不读取运行时 API Key，不运行真实漏洞研究，不上传本地项目数据。

### 8.3 Live Provider Smoke Workflow

真实 Provider 兼容性测试与 Pull Request CI、Release Workflow 完全分离：

- 文件建议命名为 `live-smoke.yml`，只接受 `workflow_dispatch` 手动触发。
- 使用 GitHub Environment `live-test`，Secret 按 Provider Profile 使用 `CYBER_AGENT_*_API_KEY` 命名空间。
- Primary、Vision 与本地开发 Key 分开创建和撤销；同一 Workflow 不要求两种模式共享 Provider。
- Secret 只进入运行 ignored live test 的单个 step，不设置为 Workflow 或所有 Job 的全局环境变量。
- 不响应 `pull_request`、Fork 或 `pull_request_target`，也不复用普通 PR 的任意输入作为模型 Prompt。
- 只运行固定的合成 smoke cases，并限制并发、超时和输出长度。
- Primary 和 Vision Smoke 可独立选择运行；组合 Smoke 仅在两条单 Provider 路径通过后执行。
- 不上传请求/响应正文、Authorization Header、测试日志、`.cyber-agent/` 或目标 Artifact。

Live Smoke 失败只表示 Provider 兼容路径需要诊断，不得通过导入 Codex CLI 登录或修改 `~/.codex` 来修复。

## 9. Dependency 管理

- 二进制项目提交 Cargo.lock。
- 日常构建和 CI 使用 `--locked`。
- 新依赖必须记录解决的问题、feature、替代方案和编译成本。
- 关闭不需要的 default features。
- 依赖更新使用独立分支/PR，不与功能混合。
- Major update 必须运行全部 Trace/Fixture 和迁移测试。
- 不自动执行大范围 `cargo update`。

从 Codex 移植代码还必须更新 `UPSTREAM_PORT_LEDGER.md` 和第三方 Notice。

## 10. Hotfix 与回滚

- Hotfix 从已发布 Tag 对应 main 提交创建 `fix/...` 分支。
- 修复后先合入 main，再发布 patch version。
- 不维护长期 hotfix branch。
- 二进制回滚使用上一 Tag/Release Artifact。
- Event Schema 问题优先发布兼容 Reader/Upcaster；不得回滚时破坏已经写入的新 Event。
- 破坏性数据迁移必须先 export 新 Stream，并保留原文件。

## 11. Git 安全检查清单

任何 commit/push/tag 前检查：

- `git status --short`
- `git diff --check`
- `git diff --cached --stat`
- `git diff --cached --name-only`
- staged 文件中没有 `/codex/`
- staged 文件中没有 `.cyber-agent/`
- 没有 `.env`、Key、Token、真实目标 Artifact 或本地 Request Snapshot
- 所有要求测试通过
- STATUS 与 Slice 一致

本清单用于防止误提交和保持可恢复历史，不是 Cyber Agent 运行时安全系统。
