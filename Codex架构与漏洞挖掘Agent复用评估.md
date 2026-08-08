# Codex 架构与漏洞挖掘 Agent 复用评估

## 1. 文档目的

本文记录对本地 Codex 开源代码的架构分析，以及面向个人漏洞挖掘 Agent 的二次开发决策。

- 本地源码目录：`codex/`
- 分析提交：`3aae5d885bac`
- 分析原则：只以本地源码为证据，不依赖外部官方资料
- 当前状态：方案参考，尚未实施代码裁剪或迁移
- 目标：保留 Codex 中优质的 Agent 基础能力，避免继承完整产品栈的通用性和复杂度

需要长期保持的事实边界：

- “适合复用”不代表代码已经迁移。
- “候选架构”不代表新 Agent 已经实现。
- Hook 可以用于策略门禁，但不能替代 Agent 主循环。
- 多模态是漏洞研究需要保留的基础能力。

## 2. 总体结论

本地 Codex 不是一个轻量 Agent 内核，而是一套通用 Agent 产品平台。它同时包含 CLI、TUI、App Server、云任务、插件、Connectors、多 Agent、跨平台沙箱、遥测、登录、配置兼容、持久化与 SDK。

对于个人漏洞挖掘专用 Agent，长期推荐方案是：

> 新建专用项目，抽取 Codex 的协议、上下文、工具、多模态、历史和传输能力，自行实现更小的漏洞研究主循环。

短期可在完整 Codex 上使用 Hook + Extension 做 PoC，验证授权边界、工具门禁、研究状态和证据验收规则。验证成熟后，再迁入专用 Agent。

不推荐直接在完整 Codex 上大量删除目录。这样会持续面对 Feature 交叉、App Server API、配置兼容、Rollout 兼容、跨平台构建和上游同步成本。

本地仓库的 `AGENTS.md` 也明确说明 `codex-core` 已经臃肿，新增能力应尽量拆到独立 crate。这支持“抽取内核能力，而不是继续向 core 堆积漏洞研究逻辑”的判断。

## 3. 总体架构

    npm 分发包装 codex-cli/
              │
              ▼
    Rust 产品入口 codex-rs/cli
      ├── TUI
      ├── Exec
      ├── App Server
      ├── MCP Server
      ├── Cloud Tasks
      └── 登录、配置、迁移、调试等命令
              │
              ▼
    App Server / InProcessAppServerClient
              │
              ▼
    ThreadManager
      └── CodexThread
           └── Session
                └── RegularTask
                     └── session::turn::run_turn
                            │
                            ├── TurnContext
                            ├── StepContext
                            ├── WorldState
                            ├── History
                            ├── Hook / Extension
                            └── ToolRouter
                                    │
                                    ▼
    Prompt
      ├── base_instructions
      ├── input: Vec<ResponseItem>
      ├── tools: Vec<ToolSpec>
      └── output_schema
              │
              ▼
    ModelClientSession
      ├── Responses WebSocket
      ├── previous_response_id 增量请求
      └── 失败回退 HTTP/SSE
              │
              ▼
    ResponseEvent
      ├── Assistant Text / Reasoning
      ├── Function / Custom Tool Call
      ├── Token Usage
      └── Completed
              │
              ▼
    ToolRouter → PreToolUse Hook → Executor → PostToolUse Hook
              │
              ▼
    Tool Output 写回 History → 再次采样
              │
              ▼
    Stop Hook → 完成 Turn 或注入 continuation 后继续
              │
              ▼
    Rollout / Thread Store / State / Compaction

关键代码入口：

- CLI 总分发：`codex/codex-rs/cli/src/main.rs`
- 线程管理：`codex/codex-rs/core/src/thread_manager.rs`
- 普通任务：`codex/codex-rs/core/src/tasks/regular.rs`
- 单 Turn 主循环：`codex/codex-rs/core/src/session/turn.rs`
- TurnContext：`codex/codex-rs/core/src/session/turn_context.rs`
- StepContext：`codex/codex-rs/core/src/session/step_context.rs`

TurnContext 保存一次 Turn 内相对稳定的状态，包括模型、推理等级、配置、权限、人格、developer instructions、输出 Schema 和动态工具。

StepContext 是一次模型采样的快照，包含当前环境、MCP Binding、ToolRouter、选中的能力目录和 AGENTS.md。一次 Turn 内的不同采样可以捕获不同 StepContext。

## 4. Prompt 不是单一字符串

Prompt 定义在：

`codex/codex-rs/core/src/client_common.rs`

核心结构：

    Prompt {
        input: Vec<ResponseItem>,
        tools: Vec<ToolSpec>,
        parallel_tool_calls: bool,
        base_instructions: BaseInstructions,
        output_schema: Option<Value>,
        output_schema_strict: bool,
    }

模型输入存在三条独立通道：

| 通道 | 主要内容 | Responses 请求字段 |
|---|---|---|
| 基础指令 | 模型级行为、人格和通用 Agent 规则 | `instructions` |
| 会话输入 | developer/user/assistant/reasoning/tool call/tool output | `input[]` |
| 工具能力 | 工具名称、说明、JSON Schema | `tools[]` |

因此，专用 Agent 不能只替换一个 system prompt。至少需要同时控制：

- Base Instructions
- Developer/Context 注入
- World State
- 工具暴露和工具 Schema
- History 归一化
- 压缩后的上下文重建

## 5. Prompt 原始来源

| 来源 | 本地位置 | 作用 |
|---|---|---|
| 模型目录 | `codex-rs/models-manager/models.json` | 模型能力、模板、人格变量、工具能力 |
| 默认基础指令 | `codex-rs/protocol/src/prompts/base_instructions/default.md` | 默认模型指令 |
| 模型模板 | `codex-rs/core/templates/model_instructions/` | 特定模型指令模板 |
| 旧模型 Prompt | `codex-rs/core/gpt_*_prompt.md` | 旧模型兼容模板 |
| 功能 Prompt | `codex-rs/prompts/templates/` | 权限、压缩、Review、Realtime、Goal |
| 配置覆盖 | `codex-rs/config/src/config_toml.rs` | instructions、developer instructions、compact prompt |
| 初始上下文组装 | `codex-rs/core/src/session/mod.rs` | 合并 developer/user contextual fragments |
| World State | `codex-rs/core/src/session/world_state.rs` | 环境、权限、AGENTS.md、模式等动态状态 |
| Turn 注入 | `codex-rs/core/src/session/turn.rs` | 用户输入、Skill、Plugin、Extension |
| Hook 上下文 | `codex-rs/core/src/hook_runtime.rs` | SessionStart/UserPromptSubmit/PostToolUse/Stop |

基础指令优先级：

1. `config.base_instructions`
2. 恢复历史中的 `session_meta.base_instructions`
3. 当前 ModelInfo 渲染后的 `instructions_template`

`{{ personality }}` 在 `ModelInfo::get_model_instructions()` 中替换。

主要配置覆盖项：

- `instructions`
- `model_instructions_file`
- `developer_instructions`
- `compact_prompt`
- `include_permissions_instructions`
- `include_apps_instructions`
- `include_collaboration_mode_instructions`
- `include_environment_context`

## 6. World State

World State 注册顺序：

1. 模型切换说明
2. Personality
3. Token Budget
4. Context Window Guidance
5. Realtime
6. AGENTS.md
7. Permissions
8. Collaboration Mode
9. Environment
10. Environment Instructions
11. Apps
12. Plugins
13. Deferred Tools
14. Extension World State
15. Multi-agent Usage Hint
16. Multi-agent Mode

World State 的关键设计：

- 每个 Section 有稳定 ID。
- 每个 Section 保存紧凑的类型化 Snapshot。
- 使用 RFC 7386 merge patch 保存变化。
- 使用 SHA-1 计算模型可见片段指纹。
- 如果 retained history 中缺少必要片段，会重新注入。
- Extension 的 `host_skills` 可特殊插到 Permissions 前。

值得直接复用的基础抽象是：

`codex/codex-rs/context-fragments/src/fragment.rs`

`ContextualUserFragment` 统一定义：

- role
- marker
- body
- render
- 是否独立消息
- 到 ResponseItem 的转换

漏洞 Agent 可以使用同一机制定义：

- AuthorizedScopeState
- TargetVersionState
- DeviceState
- AttackSurfaceState
- HypothesisState
- EvidenceState
- FindingValidationState

## 7. 单 Turn 实际时序

一次普通 Turn 的实际顺序：

1. 收集上一 Turn 的异步 Hook 结果。
2. 执行 Turn 前上下文压缩检查。
3. 从用户输入解析 Skill、Plugin、App 和 MCP mention。
4. 启动必要 MCP Server。
5. 捕获第一个 StepContext。
6. 记录完整初始上下文或 World State diff。
7. 构造 Skill、Plugin、Extension 注入，但暂不记录。
8. 执行 SessionStart Hook，其上下文立即进入历史。
9. 执行 UserPromptSubmit Hook。
10. 记录真实用户消息。
11. 记录 UserPromptSubmit Hook 的 developer additional context。
12. 记录 Skill、Plugin、Extension 注入。
13. 每次模型采样前重新计算 World State diff。
14. 调用 `History::for_prompt()`。
15. 构造 Prompt。
16. 转为 Responses 请求并流式调用模型。
17. 模型输出或工具调用写入历史。
18. 工具调用经 PreToolUse Hook、Executor、PostToolUse Hook。
19. 工具结果写回 History。
20. 如需 follow-up，再次采样。
21. 无工具后执行 Stop Hook。
22. Stop Hook 可注入 continuation prompt 并强制继续。

典型历史顺序：

    初始上下文 / World State diff
    SessionStart Hook 上下文
    用户消息
    UserPromptSubmit Hook 上下文
    显式 Skill 内容
    Plugin 内容
    Extension Turn Input
    模型输出
    工具调用
    工具结果
    下一次模型输出

## 8. History 和 Prompt 归一化

数据流：

    UserInput
      ↓
    ResponseInputItem
      ↓
    ResponseItem
      ↓
    图片/音频处理 + ID 分配
      ↓
    Durable History / Rollout
      ↓
    History::for_prompt()
      ↓
    Prompt
      ↓
    ResponsesApiRequest
      ↓
    ResponseEvent
      ↓
    ResponseItem / Tool Result
      ↓
    History

History 在发送模型前会：

- 过滤 system 和内部不可用于 API 的项目。
- 为缺少结果的工具调用补充 `aborted`。
- 删除孤立 Tool Output。
- 保证 Call/Output 配对。
- 按模型策略截断工具输出。
- 对不支持图片或音频的模型使用文本占位符。
- 使用 UUID v5 生成稳定的合成输出 ID。

稳定 ID 可以保证 retry、resume 和 Prompt Cache 的一致性。

相关代码：

- `codex-rs/core/src/context_manager/history.rs`
- `codex-rs/core/src/context_manager/normalize.rs`
- `codex-rs/utils/output-truncation/`

## 9. 模型请求和增量传输

普通 Responses 请求包含：

    {
      "model": "...",
      "instructions": "...",
      "input": [],
      "tools": [],
      "tool_choice": "auto",
      "parallel_tool_calls": true,
      "reasoning": {},
      "stream": true,
      "include": ["reasoning.encrypted_content"],
      "prompt_cache_key": "...",
      "text": {}
    }

`ModelClientSession` 是 Turn 级对象，同一 Turn 内复用：

- WebSocket
- Sticky routing
- Last request
- Last response ID

增量请求成立条件：

- 除 input 外的所有请求属性一致。
- 新 input 严格等于“旧 request input + 服务端上次 output + 新增 input”。

成立时只发送：

    previous_response_id + incremental input

WebSocket 失败后自动回退 HTTP/SSE。

值得复用：

- 请求属性一致性判断
- 稳定 Prompt Cache Key
- previous_response_id 增量算法
- WebSocket 到 HTTP 的透明回退

Responses Lite 是特殊分支：

- Tools 改写成 `AdditionalTools` developer item。
- Base Instructions 改写成 developer message。
- 顶层 `instructions` 清空。
- 顶层 `tools` 不再发送。
- 关闭 parallel tool calls。
- 删除图片 detail。

## 10. 工具架构

ToolRouter 构建顺序：

    Core Tools
      ↓
    MCP Tools
      ↓
    MCP Exposure Policy
      ↓
    Extension Tools
      ↓
    Dynamic Tools
      ↓
    Model-visible Tool Specs

工具暴露模式：

- Direct
- Deferred
- DeferredModelOnly
- DirectModelOnly
- CodeModeOnly
- Hidden

工具执行顺序：

    模型 Tool Call
      ↓
    立即持久化 Tool Call
      ↓
    Parallel / Serial Gate
      ↓
    PreToolUse Hook
      ↓
    ToolExecutor
      ↓
    PostToolUse Hook
      ↓
    Tool Result 写入 History
      ↓
    下一次模型采样

漏洞研究原生工具应通过 ToolContributor 或 MCP 实现，例如：

- 静态分析
- Semgrep/CodeQL
- APK/IPA 反编译
- 二进制反汇编
- 模糊测试
- Frida/动态插桩
- HTTP 抓包与重放
- 崩溃归档
- 证据哈希
- Finding 台账

## 11. Hook 的能力边界

适合 Hook 的能力：

- 授权目标白名单
- 域名/IP/包名/设备范围校验
- 危险命令阻断
- 出网和凭据操作门禁
- 工具参数改写
- 工具结果质量检查
- 证据完整性校验
- Candidate 到 Verified 的升级门禁
- Stop 前检查报告和复现证据

具体能力：

| Hook | 能力 |
|---|---|
| SessionStart | 初始化授权范围和研究状态 |
| UserPromptSubmit | 阻断用户输入或追加 developer context |
| PreToolUse | 阻断工具或修改参数 |
| PostToolUse | 替换模型可见结果、阻断结果、追加上下文 |
| Stop | 阻止结束并注入 continuation prompt |

重要限制：

- PostToolUse 执行时工具已经运行，无法回滚工具副作用。
- Hook 不能替换 ToolRouter。
- Hook 不能替换 History。
- Hook 不能替换 Compaction。
- Hook 不能替换 Rollout 恢复。
- Hook 不能替换主采样状态机。

因此：

- Hook 是策略、授权、审计和验收层。
- Extension 是漏洞研究原生上下文和工具层。
- 专用 Agent 内核仍需要新的主循环。

## 12. Extension

Extension API 已提供：

- ContextContributor
- World State Contributor
- TurnInputContributor
- ToolContributor
- McpServerContributor
- ToolLifecycleContributor
- TurnLifecycleContributor

相关位置：

- `codex-rs/ext/extension-api/src/contributors.rs`
- `codex-rs/ext/extension-api/src/registry.rs`

短期 PoC 可以新增一个 Security Research Extension：

    security-research-extension
    ├── AuthorizedScopeContributor
    ├── ResearchWorldStateContributor
    ├── SecurityToolContributor
    ├── EvidenceContributor
    └── FindingLifecycleContributor

## 13. 多模态

多模态应保留。

输入链路：

    LocalImage / LocalAudio
      ↓
    UserInput
      ↓
    ResponseInputItem
      ↓
    ResponseItem
      ↓
    Durable History Preparation
      ↓
    图片/音频规范化
      ↓
    History
      ↓
    Responses 请求

图片能力：

- high/auto：最大维度 2048，最多 2500 patches
- original/unified：最大维度 6000，最多 10000 patches
- low detail 不支持
- HTTP/HTTPS 远程图片 URL 会被拒绝
- `view_image` 工具复用相同历史处理链

音频能力：

- 最大 50 MiB
- 支持 wav、mp3、m4a、webm、ogg
- Token 估算约 10 tokens/秒

建议复用：

- Protocol 中的 Image/Audio ContentItem
- `utils/image`
- `utils/audio`
- Durable history preparation
- `view_image`

`ext/image-generation` 是图片生成能力，不等同于图片理解，可按实际需求移除。

## 14. 压缩和恢复

压缩策略：

    TokenBudget
      → 切换新 Context Window

    Remote V2 + Feature
      → input 末尾追加 CompactionTrigger

    Remote V1/V2
      → 远端 responses compact

    不支持 Remote
      → 模型生成本地摘要

本地摘要：

- 默认 Prompt：`prompts/templates/compact/prompt.md`
- 摘要前缀：`prompts/templates/compact/summary_prefix.md`
- 最近用户消息预算约 20K tokens
- 摘要作为最后一个 user 角色消息

上下文重注入：

- Pre-turn/manual compaction 不直接放入初始上下文。
- 清除 reference context baseline。
- 下一真实 Turn 完整重注入。
- Mid-turn compaction 把初始上下文插在最后真实用户消息前。

Rollout 主要项目：

- SessionMeta
- ResponseItem
- InterAgentCommunication
- Compacted
- TurnContext
- WorldState
- EventMsg

恢复算法：

1. 从后向前按 Turn segment 扫描。
2. 应用 ThreadRolledBack。
3. 找到最新有效 replacement history。
4. 恢复 model、comp hash、realtime、TurnContext。
5. 恢复 World State 和 Context Window UUID 链。
6. 正向重放 checkpoint 后的项目。
7. 重新处理历史媒体，但不改原 Rollout。
8. 恢复 TokenCount。
9. 下一真实 Turn 决定完整重注入或 diff。

这一设计值得借鉴，但当前实现和 codex-core 耦合较深。建议复用设计和测试思路，不直接复制全部实现。

## 15. 顶层目录职责

| 目录 | 职责 |
|---|---|
| `codex-rs/` | Rust 主工作区，显式声明 130 个 workspace member |
| `codex-cli/` | npm/Node 分发包装 |
| `sdk/` | Python、Python Runtime、TypeScript SDK |
| `.github/` | CI、跨平台构建、签名、发布、V8 和 SDK Workflow |
| `bazel/` | Bazel 平台、规则、工具链和发布目标 |
| `scripts/` | 构建、安装、打包、发布、MCP conformance |
| `patches/` | 第三方构建与跨平台补丁 |
| `third_party/` | V8、PowerShell、WezTerm、Wine |
| `tools/` | 仓库开发检查工具 |
| `docs/` | 安装、构建和贡献文档 |
| `.codex/` | 仓库本地 Codex 环境和 Skill |
| `.devcontainer/` | 开发容器 |
| `.vscode/` | 编辑器配置 |

## 16. codex-rs 分层

| 分层 | 主要目录 |
|---|---|
| 产品入口/UI | `cli`、`tui`、`exec`、`app-server*`、`mcp-server`、`exec-server*` |
| Agent 核心 | `core`、`core-api`、`protocol`、`context-fragments`、`prompts` |
| 模型传输 | `codex-api`、`codex-client`、`http-client`、`websocket-client`、`model-provider*`、`models-manager` |
| 工具执行 | `tools`、`apply-patch`、`shell-command`、`file-search`、`file-system`、`code-mode*` |
| 扩展体系 | `ext/*`、`hooks`、`plugin`、`core-plugins`、`skills`、`connectors`、`codex-mcp` |
| 沙箱权限 | `sandboxing`、`linux-sandbox`、`windows-sandbox-rs`、`bwrap`、`execpolicy`、`network-proxy` |
| 持久化 | `rollout`、`rollout-trace`、`thread-store`、`state`、`message-history` |
| 多 Agent | `agent-graph-store`、`agent-identity`、`ext/agent`、`ext/queue` |
| 多模态 | `utils/image`、`utils/audio`、`ext/image-generation` |
| 云和产品服务 | `backend-client`、`chatgpt`、`cloud-*`、`feedback`、`external-agent-migration` |
| 身份安全 | `login`、`aws-auth`、`keyring-store`、`secrets`、`process-hardening` |
| 观测诊断 | `analytics`、`otel`、`diagnostics`、`response-debug-context` |
| 通用组件 | `utils/*`、`async-utils`、`ansi-escape`、`git-utils`、`file-watcher` |

## 17. 复用分级

### 17.1 可直接复用

- `codex-protocol` 的 UserInput、ResponseItem、Tool Call 数据模型
- `context-fragments`
- `codex-tools` 的 ToolSpec、ToolExecutor、ToolExposure
- `utils/image`
- `utils/audio`
- `utils/output-truncation`
- `apply-patch`
- 必要的 shell command parsing
- Extension API 的 Contributor 接口设计

### 17.2 抽取后复用

- World State snapshot/diff
- History 工具调用配对和归一化
- WebSocket incremental request
- Hook 事件模型和 Hook Runner
- Rollout replacement-history/checkpoint
- 目标平台对应的 exec/sandbox
- Prompt Cache 稳定 ID 和请求属性比较

### 17.3 强耦合，不建议直接复用

- 完整 `codex-core`
- CLI/TUI/App Server 全产品栈
- ChatGPT 和 Cloud Tasks
- Connectors 和 Marketplace
- Guardian
- 通用多 Agent 产品逻辑
- 遥测、反馈、远程管理
- 全平台沙箱实现
- 完整配置兼容和历史迁移

### 17.4 漏洞 Agent 自行实现

- 小型 Turn/Sampling Loop
- ResearchWorldState
- Authorized Scope Policy
- SecurityToolRouter
- Evidence Store
- Finding Candidate/Verified 生命周期
- Finding Validator
- 研究预算与任务调度
- 专用报告生成
- 专用恢复和 Checkpoint 策略

## 18. 推荐的新项目架构

    cyber-agent/
    ├── crates/
    │   ├── cyber-agent-core/
    │   │   ├── turn_loop
    │   │   ├── sampling
    │   │   ├── history
    │   │   └── checkpoint
    │   ├── cyber-agent-protocol/
    │   ├── cyber-agent-context/
    │   ├── cyber-agent-tools/
    │   ├── cyber-agent-hooks/
    │   ├── cyber-agent-multimodal/
    │   ├── cyber-agent-evidence/
    │   └── cyber-agent-model-client/
    ├── extensions/
    │   ├── static-analysis/
    │   ├── mobile-analysis/
    │   ├── binary-analysis/
    │   ├── web-analysis/
    │   └── fuzzing/
    ├── policies/
    │   ├── authorization
    │   ├── destructive-actions
    │   └── evidence-validation
    └── apps/
        └── cyber-agent-cli/

ResearchWorldState 建议至少包括：

- 授权范围
- 当前目标
- 目标版本
- 设备/环境
- 攻击面
- 候选假设
- 已执行实验
- 正向证据
- 负向对照
- Candidate/Verified 状态
- 最终影响
- 剩余研究预算

## 19. 推荐实施路线

### 阶段一：Hook + Extension PoC

- 不删除 Codex 模块。
- 使用配置关闭不需要的能力。
- 用 ToolExposure 隐藏无关工具。
- 实现授权、范围、危险操作和证据门禁 Hook。
- 实现漏洞研究 World State 和专用 ToolContributor。
- 验证真实漏洞研究工作流。

### 阶段二：抽取基础模块

- 建立独立 Rust workspace。
- 固定上游 Codex 提交和 Apache-2.0 许可记录。
- 优先迁移 protocol、context fragments、tools、media、output truncation。
- 为 World State、History 和增量请求建立适配层。
- 不直接依赖完整 codex-core。

### 阶段三：实现专用内核

- 编写更小的 Turn/Sampling Loop。
- 用 ResearchWorldState 取代通用产品上下文。
- 建立 Finding Candidate/Verified 状态机。
- 建立 Evidence Store 和 Checkpoint。
- 逐步迁移已验证有效的 Hook/Extension 策略。

### 阶段四：精简产品层

- 只保留一个个人 CLI。
- 只编译当前平台需要的沙箱。
- 按需增加轻量 UI。
- 使用 GitHub Actions 构建和发布个人二进制。

## 20. 当前决策

当前确定：

- 保留多模态。
- Hook 用于策略和验收，不作为主内核。
- Extension 用于专用上下文和漏洞研究工具。
- 长期新建项目，采用“抽取/软分叉内核 + 专用上层”的模式。
- 不在完整 Codex 上进行大规模物理删除。
- 暂不提交或推送任何本地文件。

后续所有实现都应继续区分：

- 源码中已经存在的能力
- 已抽取并完成测试的能力
- 仅作为设计参考的能力
- 漏洞研究候选功能
- 已验证可用于真实工作流的功能
