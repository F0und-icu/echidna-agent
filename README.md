<p align="center">
  <img src="assets/readme-banner.svg" alt="Echidna — Local Vulnerability Research Agent. Question. Observe. Verify." width="100%">
</p>

<p align="center">
  <strong>简体中文</strong> · <a href="README.en.md">English</a>
</p>

<p align="center">
  <strong>在本地开展研究，让每一步都留下可追溯的依据。</strong>
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="#系统设计">系统设计</a> ·
  <a href="#自动任务上下文">任务上下文</a> ·
  <a href="#使用指南">使用指南</a>
</p>

---

Echidna 是面向个人使用的本地优先 Agent，以 Rust 实现，通过交互式 CLI 调用模型、
文件、命令行和网络工具。会话、工具回执与研究材料保存在项目中，便于继续工作和回看依据。

主分支 `main` 支持外接人格、工作内容和方法文件；默认提供漏洞研究 Agent，
也可用于技术写作等任务。原研究主线保留在 `codex/backup-main-2026-11-14`；
后续开发与更新以 `main` 为准。

## 能力

| 能力 | 使用方式 |
| --- | --- |
| 分析与执行 | 读写文件、搜索代码、运行命令、HTTP、后台 Job / PTY 和 MCP 工具。 |
| 自动任务上下文 | 自然切换、追问或返回旧任务，由 harness 选择相关材料。 |
| 持续工作 | 恢复会话、压缩长上下文，保留实际工具结果与来源。 |
| 可编辑 Agent | 外接人格、方法和作用域记忆，可显式启用 RSI 迭代。 |
| 图像辅助 | 使用多模态主模型，或单独配置视觉模型。 |
| 过程可查 | 查看执行回执、请求用量与调试输出，导出会话记录。 |

本地优先指数据与工具执行位于本地；使用远端模型时，请求材料会发送给该服务。
工具按当前用户权限执行，当前没有内置沙箱或执行确认层。

## 快速开始

当前主要面向 macOS，需要 Git 和 Rust/rustup；[工具链](rust-toolchain.toml)固定为 Rust 1.93.1。
以下命令在同一终端中执行。

### 1. 构建

```sh
git clone --branch main --single-branch https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release
ECHIDNA_BIN="$PWD/target/release/echidna"
```

### 2. 配置模型

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

编辑配置中的服务地址、模型和能力声明，再设置模板引用的密钥：

```sh
export CYBER_AGENT_PRIMARY_API_KEY='replace-with-your-primary-key'
```

当前使用 **HTTP/SSE + Responses** 协议；Chat Completions 接口不适用。
模板默认窗口为 272K，必须按服务实际支持的容量填写；明确支持 1M 时可配置为 1M。
程序不会自动加载 `.env`。多模态、MCP、订阅登录见[配置指南](guides/setup.md)。

### 3. 开始工作

```sh
mkdir -p ../echidna-research
cd ../echidna-research
"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`doctor` 只做本地检查，不验证密钥或服务可达性；在 Chat 提交任务后才请求模型。
外接人格使用 `chat --agent /path/to/agent`，详见 [Agent 定义](guides/agents.md)。

## 系统设计

一个 AgentLoop 负责上下文、模型采样与工具执行。每次采样固定上下文和工具计划，
实际结果先持久化，再开展依赖它的工作；恢复不会重跑旧工具。

[![Echidna system architecture](assets/architecture-overview.svg)](assets/architecture-overview.svg)

[架构说明与开发验证 →](guides/architecture.md)

## 自动任务上下文

**本地检索候选 → 模型判断任务关系 → harness 编译当前上下文。**
完整历史仍然保留；需要排除旧任务材料时，同时建立新的模型上下文。
无需手动管理任务，选择请求会增加开销，实际效果仍取决于模型判断。

[![Echidna automatic task context](assets/task-context-flow.svg)](assets/task-context-flow.svg)

[算法、交互示例与请求成本 →](guides/task-context.md)

## 日常使用

| 操作 | 入口 |
| --- | --- |
| 恢复最近会话 | `echidna --config /path/to/provider.toml chat --resume latest` |
| 查看实际请求与调试信息 | 启动 Chat 时添加 `--debug` |
| 接入外部 TUI 或 Web | 使用 `chat --output-format jsonl` 逐行读取[显示事件](guides/chat-events.md)。 |
| 停止当前轮次 | `/stop` 或 Ctrl+C；已发生的操作不会撤销。 |
| 压缩上下文 / 添加图片 / 退出 | `/compact` · `/image PATH` · `/exit` |

[完整交互、恢复、压缩与诊断说明 →](guides/runtime.md)

## 使用指南

| 指南 | 内容 |
| --- | --- |
| [配置与接入](guides/setup.md) | 模型、密钥、订阅认证、MCP、多模态与配置示例 |
| [Agent 定义](guides/agents.md) | 人格、方法、记忆、重载与可选 RSI |
| [任务上下文](guides/task-context.md) | 算法流程、任务关系、来源回读与开销 |
| [日常使用](guides/runtime.md) | 会话恢复、中断、长上下文、用量与故障诊断 |
| [架构与开发](guides/architecture.md) | 执行与持久化模型、验证命令和后续方向 |
| [配置模板](examples/README.md) | 可复制的 Provider / Agent 模板及字段说明 |
| [更新记录](CHANGELOG.md) | 功能变化与兼容性说明 |
