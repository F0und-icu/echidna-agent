# 开发环境与构建环境管理方案

## 0. 目标

建立一个个人可维护、Codex 可重复使用、CI 能复现的 Rust 开发环境，同时避免引入容器平台、云开发环境和繁重工具链。

## 1. 支持范围

Primary：

- macOS
- Apple Silicon / `aarch64-apple-darwin`
- 本地交互 CLI

Secondary：

- GitHub Actions macOS 构建。
- Linux 只在依赖成本合理时做 compile/test，不承诺首期完整 PTY/设备体验。

不要求：

- Windows 首期支持。
- Docker 作为默认开发方式。
- Nix、Bazel、Buck、Dev Container 或远程开发 Server。
- Node/Python 作为核心构建依赖。

## 2. 固定工具链

M0-S1 创建 `rust-toolchain.toml`：

```toml
[toolchain]
channel = "1.93.1"
profile = "minimal"
components = ["rustfmt", "clippy"]
targets = ["aarch64-apple-darwin"]
```

版本选择规则：

- 在 M0-S1 当天选择本机和 GitHub Actions 均可用的稳定 Rust。
- 选择后写入 exact version，不使用浮动 `stable`。
- `Cargo.toml` 同时声明 `rust-version` 作为 MSRV。
- 升级 Rust 使用独立 chore PR，运行所有测试和 release build。
- Architecture/Event Schema 不因编译器升级而改变。

## 3. 必需本地工具

基础：

- Xcode Command Line Tools
- Git
- rustup
- 固定 Rust toolchain
- cargo、rustfmt、clippy
- `rg`

建议但非核心依赖：

- `jq`：检查 JSON/JSONL。
- `gh`：用户授权后的 GitHub 仓库和 Workflow 操作。
- `cargo-nextest`：测试规模证明默认 cargo test 过慢后再引入。

不要求安装 `just`、`make` wrapper 或 xtask。开发命令优先保持为标准 Cargo 命令。

## 4. 环境预检

实现开始前只读检查：

```sh
uname -a
uname -m
xcode-select -p
git --version
rustup --version
rustc --version --verbose
cargo --version
rg --version
```

`doctor` 命令最终应报告：

- 当前平台和架构。
- Project Root 和 `.cyber-agent/` 状态。
- 配置目录。
- Model Provider 配置是否完整，但不显示 Key。
- 可用外部工具及版本。
- PTY、图片和网络能力。
- Event/Cache Schema 兼容性。
- TargetSnapshot 当前状态。

## 5. 仓库环境文件

计划创建：

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
.gitignore
.cargo/config.toml        # 只有出现稳定必要配置时
```

不创建包含真实凭据的 `.env`。如需要示例，只创建 `.env.example` 并保存变量名和说明。

Cargo.lock 必须提交，因为项目交付二进制。

## 6. 配置和目录

### 6.1 应用级配置

F1a 首先实现显式配置文件路径：CLI 使用 `--config <path>`，通用 Live Smoke 使用 `CYBER_AGENT_LIVE_CONFIG=<path>` 指向同一个 Config v1。操作系统标准配置目录的自动发现保持 Deferred，不能在不同入口各实现一套搜索规则。

F3 为每个 `[providers.<id>]` 增加可选 `connect_timeout_ms`（默认 `30000`）与 `read_timeout_ms`（默认 `300000`）；两者均为 `1..=86400000` 毫秒。连接超时约束连接建立，读取超时约束单次网络读取等待（包含响应头/后续 SSE），不是整场研究或模型总生成时限；持续发送心跳的连接可能持续更久。超时返回显式错误，不自动重发模型请求或工具执行。

可选顶层 `[prompts]` 使用 `personal_file = "personal.md"`、`project_file = "project.md"`；相对路径以配置文件所在目录为基准，不读取 Codex 文件。正式 Chat 每次采样前读取这些显式 UTF-8 普通文件，未配置即不添加，已配置但不存在/无效即报错。`doctor` 只验证配置，不读取 Prompt 内容。项目 `core.md` 通过现有 Project Memory 路径进入 ContextPlanner，受统一预算约束，不能变成第二个证据源。

F3 Live 已验证的限制：对 `incremental_input = false` 且存在活跃 completed reasoning 的会话，Core Memory 替换或当前预算无法完整保留续传时，CLI 可能以 `completed Provider reasoning continuation cannot be retained by the current Context plan` 显式退出，而不是丢弃推理或自动重试。可使用原配置 `chat --resume <id>` 恢复；上下文压力较高且具备 TargetSnapshot 时，在新的模型请求前显式 `/compact`。恢复会重新验证事实，不保证任意输入均可容纳，也不能凭空补回缺失的 ToolResult。

配置保存：

- Provider Profile。
- API Key 的本地内联值或环境变量引用；个人配置可保存明文，但不得提交真实 Key。
- 默认 Prompt/Profile。
- 个人偏好。

当前可被实现接受的最小配置示例：

```toml
schema_version = 1
active_agent_profile = "default"

[providers.primary]
binding = "http_sse"
wire_api = "responses"
base_url = "<responses-service-base-url>"
model = "<user-selected-model>"
api_key = "<user-selected-api-key>"
capability_mapping = "primary-v1"
generation = 1

[capability_mappings.primary-v1]
version = 1
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = true
incremental_input = false
usage_reporting = true
context_window = 32768

[agent_profiles.default]
primary = "primary"
vision_mode = "disabled"
generation = 1
```

示例中的 Base URL、模型名和 Capability 值必须由用户按目标服务的真实契约填写；每个 Provider 必须且只能配置内联 `api_key` 或 `credential_ref = "CYBER_AGENT_*_API_KEY"`。配置拒绝 `context_window = 0`，也不会从 URL、Model slug 或凭据来源猜测能力。当前实现只接受 `binding = "http_sse"` 与 `wire_api = "responses"` 组合。任何实现该协议的服务都复用同一个 Adapter；仅修改 URL、Model、凭据来源或 Capability Mapping 不需要新增厂商 binding。Chat Completions 或改变 wire 语义的“兼容”接口仍明确不支持。

`doctor` 不发起 Provider 请求，也不显示 Key；内联配置会被正常解析：

```sh
cargo run --locked -- --config /absolute/path/to/cyber-agent.toml doctor
```

### 6.2 项目级配置

目标项目的 `.cyber-agent/` 保存项目状态。它不属于 Cyber Agent 源码仓库。

### 6.3 环境变量

建议约定：

| 变量 | 用途 | 默认 |
|---|---|---|
| `CYBER_AGENT_PRIMARY_API_KEY` | 示例 Primary Profile 的专用凭据；实际名称由 `credential_ref` 精确选择 | 未设置 |
| `CYBER_AGENT_TEXT_API_KEY` | 可选文本 Primary 的另一独立凭据引用 | 未设置 |
| `CYBER_AGENT_VISION_API_KEY` | 可选 Vision Provider 的专用凭据 | 未设置 |
| `CYBER_AGENT_LIVE_CONFIG` | 通用 Live Smoke 使用的显式 Config v1 路径 | 未设置 |
| `CYBER_AGENT_LIVE_TESTS` | 显式启用 live provider test | `0` |
| `CYBER_AGENT_TEST_TMP_ROOT` | 测试临时目录覆盖 | 系统 temp |
| `RUST_LOG` | 本地开发日志过滤 | 关闭或 warn |
| `NO_COLOR` | 禁用彩色 CLI | 未设置 |

应用审计和开发日志不上传。测试输出不得打印 API Key 或完整 Authorization Header。

### 6.4 Provider 凭据隔离

Cyber Agent 的 Provider 身份和当前 Codex CLI/ChatGPT 登录是两条互不回退的凭据链：

- 不读取或修改 `~/.codex`、`CODEX_HOME`、Codex OAuth/session data 或 Codex CLI 登录状态。
- 不调用 `codex login`、`codex logout`，也不把 Codex CLI 当作 credential helper。
- Responses Adapter 只接收 Composition Root 为该 Provider 解析的显式凭据：本地内联 `api_key` 或以 `CYBER_AGENT_` 命名空间隔离的引用。两者都不标识模型或厂商；引用不存在时返回“未配置”或跳过对应 live test。
- 不自动读取通用 `OPENAI_API_KEY`。即使开发机设置了该变量，也不能成为 Cyber Agent 的隐式凭据。
- 引用式凭据按需解析：Mock、Fixture、Replay、编译和普通测试不触发真实环境 Key 读取；内联 Key 随显式配置文件解析进入本地进程，但只有 Provider stream 会把它用于请求。
- 本地开发 Key 与 GitHub Actions CI Key 分开创建、轮换和撤销。Primary 与 Vision 可以使用不同 Provider/Key；同一 Provider 承担两个角色时也允许分别配置可撤销 Key。

个人本地 Provider 配置可以直接保存 `api_key`，不强制文件权限、Keychain 或环境变量中转；该文件的存放和备份由用户负责，但不得作为真实仓库 fixture 提交。引用式配置仍可用于 CI。后续如接入 macOS Keychain，必须使用独立 service 名称且不得查询 Codex 使用的 Keychain 项。

Live Test 的子进程注入形式为：

```sh
env \
  CYBER_AGENT_LIVE_CONFIG='/absolute/path/to/cyber-agent.toml' \
  CYBER_AGENT_PRIMARY_API_KEY='<injected-by-local-secret-source>' \
  CYBER_AGENT_LIVE_TESTS=1 \
  cargo test --locked --test responses_http_adapter \
    synthetic_primary_live_smoke_uses_provider_composition -- --ignored --exact
```

占位符由本地秘密存储在执行时注入；不得把真实值保存到命令示例或仓库。对子进程的临时注入不修改父进程或当前 Codex 的登录状态。

未来混合模型 Live Test 仍按角色分别注入，且必须复用同一个 `CYBER_AGENT_LIVE_CONFIG` 装配入口；缺少 Vision Key 不阻塞文本 Primary 测试：

```sh
env \
  CYBER_AGENT_LIVE_CONFIG='/absolute/path/to/cyber-agent.toml' \
  CYBER_AGENT_TEXT_API_KEY='<injected-text-test-key>' \
  CYBER_AGENT_VISION_API_KEY='<injected-vision-test-key>' \
  CYBER_AGENT_LIVE_TESTS=1 \
  cargo test --test live_model_composition -- --ignored
```

不要把两个角色强制绑定到同一变量，也不要在 Primary 失败时自动尝试 Vision Key 对应的 Provider。

## 7. Cargo Profile

### 7.1 dev/test

- 优先编译速度和调试信息。
- 测试使用临时目录，不写真实项目。
- 单元测试默认无网络、无真实设备、无 API Key。

### 7.2 release

- 使用 Cargo release 默认作为起点。
- 只有测量证明需要时才调整 LTO、codegen-units、panic 策略或 strip。
- 不以减小几个 MB 为理由引入复杂构建脚本。
- Release binary 必须支持 `--version` 和 `doctor`。

## 8. 标准命令

开发循环：

```sh
cargo fmt --all
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Gate：

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
```

Release：

```sh
rustup target add aarch64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin
shasum -a 256 target/aarch64-apple-darwin/release/cyber-agent
```

在 M0 之前这些命令没有可运行对象，不能以缺少 Cargo.toml 判断环境失败。

## 9. 测试环境隔离

- 使用系统临时目录或 `tempfile` 创建测试 Project Root。
- 每个测试创建独立 `.cyber-agent/`。
- 测试不能读取用户真实全局 Memory。
- 测试 Provider 使用 Mock、HTTP/SSE Fixture 或经过脱敏的 Replay Event。
- 测试 Device/PTY 使用 fake 或可控子进程。
- Live Test 使用独立标记并默认 ignored；只有 `CYBER_AGENT_LIVE_TESTS=1` 和专用凭据同时存在时才能运行。
- Live Test 只使用固定合成 Prompt，不加载真实漏洞目标、Project Memory、历史 Session 或 Artifact。
- Responses Full-only reasoning Tool Loop 必须由 Config v1 显式声明 `incremental_input = false`；测试代码不得按 Provider/Model 名覆盖 Capability。
- completed reasoning 同时含明文 `reasoning_text` 和 opaque encrypted sidecar 时只测试通用明文 replay；encrypted-only 仍是显式协议失败，任何 opaque 值不得进入日志或 Core。
- Live Test 限制用例数、并发、超时和输出长度，只验证 Provider 编解码、Streaming、Tool Call 与 Usage 的最小兼容性。
- 凭据解析单元测试使用注入的 Fake Env/Credential Source，不调用 `std::env::set_var` 改变并行测试进程的真实环境。
- Contract Test 必须证明 `OPENAI_API_KEY`、`CODEX_HOME` 和 Codex 登录文件不会被当作回退凭据。
- VisionMode Resolver 使用 Fake Capability Mapping 穷举，不读取网络或凭据。
- 单一多模态 Primary 测试必须证明 Secondary 调用计数为零。
- 文本 Primary + Vision 测试必须证明初始图片在 Primary 首次采样前产生结构化 Observation。
- Primary/Vision ProviderSession、Response ID 和 IncrementalBaseline 不得交叉。
- Shadow Eval 默认使用 Replay ToolExecutor。
- 失败测试保留必要的临时路径提示，但不复制敏感内容到 CI Artifact。

测试矩阵：

| 层次 | Provider | 网络 | 凭据 |
|---|---|---|---|
| Unit/Port Contract | Mock/Fake | 无 | 无 |
| Adapter Integration | 本地 HTTP/SSE Fixture | 仅 loopback | 无 |
| Replay Regression | ReplayModelProvider | 无 | 无 |
| Primary Live Smoke | 真实 Primary Provider | 有 | Primary 专用测试 Key，显式 opt-in |
| Vision Live Smoke | 真实 Vision Provider | 有 | Vision 专用测试 Key，显式 opt-in |
| Composition Live Smoke | 两个已验证 Provider | 有 | 两个角色分别注入，显式 opt-in |

## 10. External Security Tools

以下工具不是 Core 构建依赖，由 `doctor` 和 `inspect_environment` 探测：

- rg、git、curl
- adb、frida
- apktool、jadx
- gdb、lldb、radare2
- nmap
- 用户自定义脚本

规则：

- 缺少领域工具不阻止 Cyber Agent 编译。
- 固定 DomainProfile/ResearchPhase ToolExposurePlan 声明当前建议暴露的工具；`inspect_environment` 报告实际可用性。
- Tool Capability Cache 保存路径、版本和探测时间，但可删除重建。
- 不由 Cyber Agent 自动下载大型漏洞工具链。

## 11. Dependency 选择原则

引入第三方 crate 前回答：

1. 它解决哪个当前 Slice 的具体问题？
2. 标准库或已有依赖为什么不足？
3. 是否能关闭默认 features？
4. 是否增加 native/system dependency？
5. 是否影响 macOS aarch64？
6. 是否把 Web/Server/Cloud/Sandbox 产品层带入默认二进制？
7. 编译时间和二进制体积变化是什么？
8. License 是否兼容，是否需要 Notice？

初期可能需要的能力类别，而不是预先锁定的 crate 清单：

- CLI parser
- async runtime
- serde JSON/TOML
- HTTP streaming
- error handling
- hashing
- UUID/time
- portable PTY
- image validation/resize
- OS config directories

依赖选择在相应 Slice 做小型 ADR 或 STATUS 决策，不提前一次性加入。

## 12. CI 环境

CI 使用 `rust-toolchain.toml`，不单独维护另一 Rust 版本。

要求：

- 使用 `--locked`。
- 不访问真实 Provider。
- 不要求 API Key。
- 不运行真实扫描、设备和 PTY 研究任务。
- macOS Job 构建 aarch64 target。
- CI Cache 只是加速，删除后必须正常构建。
- Actions 升级与 Rust 升级独立进行。

普通 PR/main CI 只运行 Mock、Fixture 和 Replay 测试，Workflow 中不存在 API Secret。真实 Provider 兼容性使用另一个手动 `workflow_dispatch` Live Smoke Workflow：

- 使用独立 GitHub Environment `live-test`。
- Environment Secret 使用对应 Profile 的 `CYBER_AGENT_*_API_KEY` 名称，且只注入需要该 Provider 的 Live Test step。
- Primary、Vision 和本地开发 Key 分别可撤销；不要求两个角色共享 Provider 或 Secret。
- 不由 `pull_request`、Fork 或 `pull_request_target` 触发。
- 固定合成输入，设置低并发、Job 超时和少量 smoke cases。
- 不上传请求/响应正文、Authorization Header、Project 数据或测试日志 Artifact。

## 13. 构建可复现性边界

本项目追求“相同源码、Cargo.lock、Rust toolchain 和 target 可以重新构建并通过测试”，不在 MVP 声称字节级 reproducible build。

Release 通过以下内容追踪：

- Git Tag/Commit。
- Cargo.lock。
- rust-toolchain.toml。
- target triple。
- GitHub runner image/architecture。
- Binary SHA-256。

## 14. 本地故障诊断顺序

1. `rustup show active-toolchain`
2. `rustc --version --verbose`
3. `cargo metadata --locked --no-deps`
4. `cargo check --locked --all-targets`
5. 检查 Xcode CLT 和 linker。
6. 检查目标架构和 native dependency。
7. 只清理明确的 `target/` 构建输出，不删除项目 `.cyber-agent/` 或 Cargo.lock。

不得使用 `git reset --hard`、删除仓库或批量清理用户目录来解决环境问题。
