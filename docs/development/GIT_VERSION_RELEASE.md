# Git、版本与发布管理方案

## 0. 当前边界

- GitHub 私有主仓库：`F0und-icu/echidna-agent`；旧 `F0und-icu/cyber-agent` 保留 core 基线。
- E0 从 `eee836d056bf71a008c5d29d0bf94c158809e687` 初始化 main 和更名前快照分支。
- 后续按 Slice 独立提交；当前精确分支以 Git/STATUS 为准，迁移见 ECHIDNA_MIGRATION.md。
- E0 新库初始化采用验证后快进 main，Actions 关闭；不宣称跑过 CI、不创建 Release。
- 当前 binary 为 `echidna`；短版本 `-V` 用于包版本校验，`--version` 另列构建时 HEAD。
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
docs/architecture-v1-3
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
- Lean Review：语义所有者、复用点、新 `pub`/依赖、测试布局、删除/Deferred 项和治理债务。
- 风险与回滚方法。

合并条件：

- Slice Gate 全部通过。
- GitHub Actions 必需 Job 通过。
- STATUS 已更新。
- `docs/development/LEAN_CODE_GOVERNANCE.md` 的 Slice Lean Gate 已通过。
- `src/` 没有就地测试，永久回归测试保留在顶层 `tests/`。
- 没有无消费者抽象、重复语义实现或未解释的公共 API/依赖增长。
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
| `0.5.0` | M3 Context、Compaction、固定领域 Profile、多模态 |
| `0.6.0` | M4 Replay、Research Control、Observation、真实 Eval |
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

- `Architecture 1.0.0` 是不可变基础架构封存。
- `Architecture 1.1.0` 是能力驱动 Primary/可选 Vision 模型组合增量封存。
- `Architecture 1.2.0` 是稳定研究人格、DomainProfile、ResearchPhase 和工具准确率增量，保持不可变。
- `Architecture 1.3.0` 是不可变 Sampling Step、Context 来源、Evidence Spine 和确定性研究引导增量；当前仅本地冻结，待单独授权后远端封存。
- `Architecture 1.4.0` 是 ToolFailure 执行边界与持久 Trace 准确率可观测性增量；当前仅本地冻结，远端封存未授权。
- 对既有架构版本的兼容澄清递增对应 patch 版本。
- 新增可选能力且不改变核心事实源发布 `1.x.0`。
- 本地冻结只禁止无 ADR 的继续改写，不等于 Git/远端封存；只有追加 `docs/architecture/SEALED_MANIFEST.md` 的精确 Hash 记录并完成授权发布后，才称为远端封存。
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
- `echidna -V` 与 Tag 一致；`--version` 的 source base 另用于定位源码基线。
- CHANGELOG 已从 Unreleased 移入版本节。
- Migration Notes 完整。
- Artifact 中不存在 API Key、`.cyber-agent/`、目标文件或本地日志。

## 8. GitHub Actions 实现

### 8.1 Pull Request CI

`.github/workflows/ci.yml` 由 `pull_request`、main push 或同仓库
`workflow_call` 触发。为了减少个人仓库的 Actions 启动和重复编译成本，
它只有一个 macOS Job，按顺序执行：

1. checkout 并安装 `rust-toolchain.toml` 固定工具链；
2. format、生产源码测试布局、Provider composition 和单 Responses owner 静态检查；
3. locked Clippy、全部默认启用的离线 Mock/Fixture/Replay 测试和 locked host build；
4. 调用 `scripts/package-macos-arm64.sh` 完成 locked arm64 release build、二进制白名单归档、架构检查和 SHA-256 自校验；
5. 上传七天保留的验证产物，供人工下载或同一次 Release workflow 使用。

普通 CI 不引用 GitHub Secret，不设置 `CYBER_AGENT_LIVE_TESTS`，所以两个
ignored Live Smoke 不会发送远程请求。分发脚本只允许归档一个顶层目录和
其中的 `echidna` 二进制；配置、`.cyber-agent/`、日志和目标数据没有
进入包的路径。

Actions 必须固定到明确版本；条件允许时固定 commit SHA。升级 Action 作为独立 chore PR。

### 8.2 Release Workflow

`.github/workflows/release.yml` 只由匹配 `v*` 的 Tag 触发。它先通过
`workflow_call` 复用上面的完整离线 Gate 和同一份构建产物，然后验证：

- Tag 是 annotated tag；
- Tag 等于 `v<Cargo package version>`；
- Tag commit 位于远程 main 历史；
- 下载产物再次通过同一分发脚本的内容白名单、Mach-O arm64 和 SHA-256 校验。

全部通过后才用 GitHub 当次短期 Token 创建 Release，并上传
`echidna-<version>-aarch64-apple-darwin.tar.gz` 与 `SHA256SUMS`。
M5-S2 只建立此路径，不自动创建第一个 Tag 或 Release。

Workflow 不读取运行时 API Key，不运行真实漏洞研究，不上传本地项目数据。

### 8.3 Live Provider Smoke Workflow

真实 Provider 兼容性测试与 Pull Request CI、Release Workflow 完全分离：

- 文件固定为 `.github/workflows/live-smoke.yml`，只接受 `workflow_dispatch` 手动触发。
- 使用 GitHub Environment `live-test`，当前固定 Primary lanes 只读取 CI 专用 `CYBER_AGENT_PRIMARY_API_KEY`。
- Secret 只进入运行 ignored live test 的单个 step，不设置为 Workflow 或所有 Job 的全局环境变量。
- 不响应 `pull_request`、Fork 或 `pull_request_target`，也不复用普通 PR 的任意输入作为模型 Prompt。
- 手动输入只选择现有 `primary_stream` 或 `shadow_tool_loop` 固定合成用例，以及 Config v1 的 URL、Model、Context Window、incremental 和 usage capability；它们不改变测试 Prompt。
- Workflow 先生成不含 Secret 的严格引用式 Config v1，再仅在所选测试 step 解析凭据；限时、串行且不上传正文或测试产物。
- 不上传请求/响应正文、Authorization Header、测试日志、`.cyber-agent/` 或目标 Artifact。

当前 workflow 不伪造一个只发文本的“Vision smoke”。正式 Direct/Secondary
图片闭环已有 F2a 双模式 Live 验收；将其自动化需要先出现一个固定、低成本、
可独立 ignored 的图片协议用例，并继续分别使用 Primary/Vision 专用 Key。

Live Smoke 失败只表示 Provider 兼容路径需要诊断，不得通过导入 Codex CLI 登录或修改 `~/.codex` 来修复。

## 9. Dependency 管理

- 二进制项目提交 Cargo.lock。
- 日常构建和 CI 使用 `--locked`。
- 新依赖必须记录解决的问题、feature、替代方案和编译成本。
- 同一职责只保留一套 HTTP、SSE、异步、图片、缓存或序列化技术栈。
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
