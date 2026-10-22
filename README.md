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
  <a href="#系统设计">系统设计</a> ·
  <a href="#能力">能力</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#日常使用">日常使用</a> ·
  <a href="#下一步">下一步</a> ·
  <a href="#文档">文档</a>
</p>

---

Echidna 是面向个人使用的本地优先漏洞研究 Agent。你给出目标、范围和问题，
它通过文件、命令行和网络工具推进分析，将会话、工具执行回执与研究材料保留在项目中，
支持继续研究、检查过程和导出记录。

项目由 Rust 实现，以交互式 CLI 为入口。源码、Web、移动端与二进制研究共用一个
AgentLoop；领域与阶段指导帮助组织研究方法，具体工具链由你的环境提供。

<a id="system-design"></a>

## 系统设计

Echidna 将单个研究 Agent 循环与可检查的本地工作记录结合起来。
用户定义任务和范围；运行时负责装配上下文、请求模型推理、执行通过校验的工具调用，
并保留继续研究或复查过程所需的观察记录。

[![Echidna 系统设计：研究输入、使用不可变采样计划的单 Agent 循环，以及持久化本地记录。](assets/architecture-overview.svg)](assets/architecture-overview.svg)

*图 1. Echidna 的执行与持久化模型。(a) 用户输入、研究指导和项目上下文来自不同来源。
(b) 单个 AgentLoop 通过不可变采样计划关联模型推理与工具派发。
(c) 固定持久化阶段保留会话事件、请求审计、原始材料和项目记录。
图中省略了具体失败路径与可选图像路由，不包含未来的任务编排。*

**每次采样固定一份计划。** 上下文装配同时考虑来源信息与可用上下文预算。
`SamplingStepPlan` 将该上下文、向模型提供的工具 Schema 和显式模型能力绑定在一起。
同一份已解析工具计划负责校验和派发模型的工具调用，使模型可见的工具与实际可执行工具保持一致。

**先建立检查点，再开展依赖它的工作。** 请求审计先于模型推理；已接受的模型采样结果
先写入检查点，再执行依赖它的工具调用。原始进程输出先发布，再保存执行回执；
配对的工具结果先写入检查点，再进入下一次模型采样。工具执行在单个 Agent 循环中
按资源约束调度。

**保留观察记录，按需派生上下文。** 原始材料与只追加事件保留实际观察到的内容。
Memory、摘要、缓存和图像观察各自保留其用途与来源，不会自动成为已验证的发现。
显式恢复会话时，先校验持久化历史，再继续执行；未知的执行影响仍明确保持为未知。

这种分工让模型能力和研究方法可以持续演进，同时将执行、请求预算和恢复保留在本地控制之下。
配置辅助视觉 Provider 时，它在同一个 Agent 生命周期内通过受限的辅助路径工作。
运行时始终保持单个 AgentLoop 与统一的 Session 管理权。

## 能力

| | 可以做什么 |
| --- | --- |
| **分析与执行** | 搜索和读取代码，编辑文件、应用补丁，运行命令、发送 HTTP 请求，使用后台任务与 PTY。 |
| **持续研究** | 保留项目上下文与会话，显式恢复已有 Session；满足前提时压缩上下文，继续长任务。 |
| **随时介入** | 在终端中停止当前 Turn、补充指令或纠正方向，查看真实工具进度和输出。 |
| **回看依据** | 查看工具执行回执、原始材料引用与请求审计，导出 Markdown / JSON 会话记录。 |
| **积累方法** | 使用项目 Markdown Memory 和显式 Personal / Project 指导文件，保存约束、决策与失败路径。 |
| **图像辅助** | 按明确配置使用主模型或可选视觉模型处理图像，保留图像来源与观察关系。 |

**本地优先**指项目数据与工具执行位于本地；配置远端模型时，请求中的材料会发送给该 Provider。
图像支持取决于能力配置，模型观察和回答仍需用实际证据验证。

## 快速开始

当前主要面向 macOS。需要 Git 和 Rust/rustup；仓库通过
[`rust-toolchain.toml`](rust-toolchain.toml) 固定 Rust 1.93.1。
外部分析工具按任务需要自行准备。

### 1. 构建

```sh
git clone https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release

ECHIDNA_BIN="$PWD/target/release/echidna"
"$ECHIDNA_BIN" --version
```

后续命令在同一个终端执行，继续使用 `ECHIDNA_BIN`。

### 2. 配置模型

按使用方式选择完整模板，保存到代码仓库之外：

| 使用方式 | 完整模板 | 图像处理 |
| --- | --- | --- |
| 纯文本模型 | [provider.toml](examples/provider.toml) | `disabled`，不接收图像 |
| 单个多模态模型 | [provider-multimodal.toml](examples/provider-multimodal.toml) | `primary_direct`，主模型直接接收图像 |
| 文本主模型 + 独立视觉模型 | [provider-text-vision.toml](examples/provider-text-vision.toml) | `secondary_tool`，视觉模型分析图像，主模型使用其观察结果继续研究 |
| 文本模型 + 远程 MCP 工具 | [provider-mcp.toml](examples/provider-mcp.toml) | `chat` 启动时发现延迟暴露的远程工具 |

下面以纯文本模板为例；需要图像能力时，将复制命令的源文件换成对应模板：

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

编辑该文件，填写服务地址和模型，并按服务实际能力核对窗口和各项能力声明。
模板中的地址、模型名与 32K 窗口都是示例；`cp -n` 保留已有文件。
这里的路径由命令显式传入，程序不会自动发现它。

当前接入 **HTTP/SSE + Responses** 协议。Chat Completions 接口不适用。
分开配置时，主模型和视觉模型的服务都需要支持此协议。

#### 环境变量与密钥

模板通过 `credential_ref` 读取密钥；服务地址、模型名和能力声明仍写在 TOML 中。
在启动 Echidna 的同一个终端设置：

```sh
export CYBER_AGENT_PRIMARY_API_KEY='replace-with-your-primary-key'
# 仅在使用独立视觉模型时设置。
export CYBER_AGENT_VISION_API_KEY='replace-with-your-vision-key'
```

对应的 Provider 配置使用 `credential_ref = "CYBER_AGENT_PRIMARY_API_KEY"`，
视觉 Provider 使用 `credential_ref = "CYBER_AGENT_VISION_API_KEY"`。
两者独立读取，各用一个变量名；程序不会回退到另一个角色的密钥或 `OPENAI_API_KEY`。
自定义名称须符合 `CYBER_AGENT_*_API_KEY` 格式，且仅含大写字母、数字和下划线。

程序**不会自动加载 `.env` 文件**。如果使用自行维护的可信 env 文件，需先在 shell 中加载并导出变量。
也可以在本地 TOML 中用 `api_key = "…"` 替换 `credential_ref`；每个 Provider 必须且只能选择一种。
真实密钥及配置保存在仓库之外，不要提交到 Git。

#### 可选远程 HTTP MCP 工具

使用 [MCP 模板](examples/provider-mcp.toml)，在 `mcp_servers` 下声明一个或多个远程
Streamable HTTP 工具服务器。每个服务器需要绝对 HTTP(S) `url`，以及
`protocol_version = "2025-11-25"` 或 `protocol_version = "2026-07-28"` 之一。
可选 `timeout_ms` 默认为 60000。

如果服务器需要 Bearer 认证，只能使用专用引用，例如
`CYBER_AGENT_MCP_EXAMPLE_API_KEY`；不得复用模型 Provider 密钥或订阅账户。在启动
Chat 的同一个终端中导出它：

```sh
export CYBER_AGENT_MCP_EXAMPLE_API_KEY='replace-with-your-MCP-key'
```

`doctor` 不读取 MCP 凭据，也不会连接 MCP 服务器。Chat 启动时会按配置名顺序各解析
一次凭据并发现有限工具目录；该目录在本进程内保持不变，压缩和 `/resume` 也复用它。
远程工具初始为延迟项，通过已有工具搜索流程暴露。它们的 schema 与参数按值传给服务端，
MCP 输出在核验前仍只是传输输出。远端断线或结果不完整会停止本轮，不会虚构收据或重放调用。

#### 可选本地 stdio MCP 工具

同一个 [MCP 模板](examples/provider-mcp.toml) 也支持 Unix stdio 服务器，每项在 `url`
和 `command` 中选一个。stdio 当前使用 `protocol_version = "2025-11-25"`：

```toml
[mcp_servers.local]
command = "python3"
args = ["/absolute/path/to/server.py", "--transport", "stdio"]
protocol_version = "2025-11-25"
env = { MY_TOOL_SETTING = "explicit-value" }
timeout_ms = 60000
```

裸命令通过 `PATH` 查找；相对命令路径和可选 `cwd` 相对配置文件所在目录解析，默认
工作目录也是该目录。参数和环境变量保持字面值，不经过 shell 或 `$VAR` 展开。
子进程只继承 `PATH`、`HOME`、`USER`、`LOGNAME`、`TMPDIR`、`TMP`、`TEMP`、`LANG`、
`LC_ALL`、`LC_CTYPE`、`TZ`，再叠加显式 `env`，不会隐式继承 Provider/vendor 密钥。
stdio 不接受 HTTP 的 `credential_ref`；真实环境密钥留在仓库外配置。

`doctor` 不启动子进程。Chat 发现工具后复用同一连接；每个 JSON-RPC 换行帧最多 2 MiB，
每次请求的收发总量最多 8 MiB，目录保持 64 页、1024 项、8 MiB 限制。
stderr 持续排空，只保留字节计数和 I/O 错误类型。取消、超时、EOF 或协议错误会关闭
连接并终止进程组，不自动重启或重放；已发送而没有最终结果的调用仍按未知执行结果处理。
退出时同步回收直接子进程，并终止同组后代；主动脱离该进程组的后代不在此清理范围。

#### ChatGPT 订阅登录

使用 [订阅模板](examples/provider-chatgpt-subscription.toml)，在仓库外保存自己的配置。
将 `auth_store` 改成不经过符号链接的绝对路径私有目录（父目录须已存在），并保留订阅配置中的官方 API 地址。
`chatgpt_subscription` 与 `api_key`、`credential_ref` 互斥。Echidna 自己发起 OAuth
授权，不读取 Codex 或浏览器已有的登录凭据。

```sh
echidna --config /absolute/path/subscription.toml auth login --provider chatgpt
echidna --config /absolute/path/subscription.toml auth status --provider chatgpt
echidna --config /absolute/path/subscription.toml auth models --provider chatgpt
```

登录会打开浏览器；加 `--no-browser` 可改为手动打开输出的授权链接。
浏览器须能访问运行 Echidna 的回环回调地址；跨机器使用时需要转发对应端口。
`status` 只检查本地状态；`models` 使用该账户向服务端查询模型。把返回的模型 slug
填入 `model`，核实并填写能力及窗口后，即可用同一配置运行 `doctor` 和 `chat`。
模板的 32K 窗口只是显式示例。HTTP 订阅路径要求 `incremental_input = false`，
不能设置 `max_output_tokens`。登录成功和离线 doctor 不代表账户已获模型使用资格。

```sh
echidna --config /absolute/path/subscription.toml auth logout --provider chatgpt
```

访问令牌在调用前按需刷新；注销会尝试撤销授权并清除本地令牌，结果会说明远端撤销
是否确认。请将认证目录保存在仓库和研究项目之外，目录/文件仅允许当前用户访问。
订阅额度与服务端授权受 [官方预览限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)
约束，失败不会自动切换为付费 API 或重跑工具。其它供应商的非 API Key 订阅尚未接入。

#### Codex 兼容订阅登录

使用 [Codex 兼容模板](examples/provider-codex-subscription.toml)，显式选择
`codex_subscription` 并保留 `https://chatgpt.com/backend-api/codex` 地址。
它与上述 SIWC 以及两种 API Key 配置互斥。请使用独立的私有 `auth_store`，
不要与 SIWC 共用；原有 API Key、Qwen、DeepSeek 配置无需修改。

如需声明目录兼容版本，可在 `codex_subscription` 内设置
`catalog_client_version = "major.minor.patch"`。缺省值保持现有 `0.144.0`；
显式值只会同时改变目录请求的 `client_version` 查询参数和 `version` 请求头，
不是已安装 Codex 版本声明，也不保证返回新模型 ID 或账户访问资格。

沿用上面的 `auth login/status/models/logout` 命令，将配置路径与 `--provider`
改成新模板中的值。Echidna 自行完成浏览器授权，使用自己的令牌存储；不读取
Codex 已有登录。回调使用本机端口 1455，端口占用时使用 1457。
若目录返回 HTTP 401，即使本地状态显示尚未到期，也可显式刷新这份自有授权：

```sh
echidna --config /absolute/path/codex.toml auth refresh --provider codex
```

此命令仅刷新 Codex 订阅凭据，返回本地状态，不打开浏览器或重发目录/模型请求。
刷新成功后再显式执行选择命令；若刷新被永久拒绝，需要重新登录。
`status` 只报告本地记录，不保证服务端仍接受授权。SIWC 和 API Key 保持原有流程。

可用交互选择器查看目录、选择模型与推理强度，并保存到当前配置：

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex
```

也可指定目录返回的模型和该模型支持的强度：

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex \
  --model gpt-6-astra --reasoning-effort xhigh
```

执行强度支持目录声明的有效字符串，包括未来新增的值；交互选择以当前模型目录为准。
直接配置的 `reasoning_effort` 接受 1–64 个 ASCII 字母、数字、下划线或连字符。
省略强度时使用服务端默认行为；显式 `none` 是不同的请求值，不等于省略。
保存会为该 Provider 复制独立能力映射，保留其他模型、注释和凭据。
配置锁协调同时运行的 Echidna 选择命令；保存期间请勿用其他编辑器同时修改该配置。
加上 `--apply-capabilities` 可同时导入该模型目录中存在的默认上下文窗口、文本/图像
输入能力及摘要参数支持声明。缺失字段或未知窗口（包括 `null`）保留当前配置；`max_context_window` 仅展示为
元数据，不会代替默认窗口。导入仍需通过现有配置和输出预算校验，失败不覆盖原文件。
不加此选项保持原有选择行为；工具等其余能力仍须显式核实，不按模型名推断。
`doctor` 可离线查看生效的强度，下一次启动 `chat` 使用保存的设置。
该兼容链路依据固定版本 Codex 源码实现，实际账号和服务端支持须通过实测确认。
失败不会切换模型、付费 API 或自动重跑工具。

Codex 链路默认请求服务端可用的推理摘要，在 stderr 上标注“推理摘要”，包括 release
构建。这是提供方返回的摘要，不是完整内部推理；摘要为空时不会补造内容。摘要不进入
对话正文，也不替代加密续接。推理强度与摘要展示独立，设置 `high` 不保证返回摘要。
若服务端不接受摘要参数，可在 `codex_subscription` 中显式设置
`supports_reasoning_summary_parameter = false`；省略或设为 `true` 保持默认请求。

#### 单个多模态模型

使用 [多模态模板](examples/provider-multimodal.toml)，由同一个主模型处理文本、工具调用和图像。
其 `[capability_mappings.primary-v1]` 声明 `image_input = true`，组合配置为：

```toml
[agent_profiles.default]
primary = "primary"
vision_mode = "primary_direct"
generation = 1
```

只需设置主模型密钥，无需额外配置视觉 Provider。

#### 文本与视觉模型分开配置

使用 [双模型模板](examples/provider-text-vision.toml)，分别填写 `[providers.primary]`
和 `[providers.vision]` 的 `base_url`、`model`、`credential_ref` 与 `capability_mapping`。
两个模型可以使用不同服务，各自声明上下文窗口和能力。关键组合配置为：

```toml
[agent_profiles.default]
primary = "primary"
vision = "vision"
vision_mode = "secondary_tool"
generation = 1
```

主模型负责推理与工具调用；视觉模型负责图像分析，将观察结果交给主模型。
两者仍在同一个 AgentLoop 中协作。主模型必须支持文本输入、流式输出和工具调用；
视觉模型必须支持文本与图像输入、流式输出和结构化输出，不要求工具调用能力。

`vision_mode = "auto"` 也是可选项：根据**声明的能力**优先使用主模型的图像能力，
否则使用已配置的视觉模型；两者都不支持时拒绝图像输入。它不会探测模型能力，也不是失败后的自动切换。

能力声明需要匹配实际服务；设置 `image_input = true` 不会让纯文本模型具备图像能力。
`context_window` 分别填写各模型的真实窗口；`[runtime].model_request_limit_per_turn`
默认为 `32`，是每轮主模型与视觉模型共享的请求次数上限。
`[runtime].auto_compact` 默认为 `false`。显式设为 `true` 后，Chat 只会在正常完成的
Turn 之后，用包含该最终回复的下一 Turn 小探针评估 Planner 压力；达到既有阈值时复用
耐久压缩流程。成功后下一次 Primary 请求为 `CompactionReset Full`，并清除该已完成
边界的短暂 Provider 续接状态。探针不代表 Provider 用量，也不保证任意下一条输入可容纳；
取消、停止、错误和未完成 Turn 不会自动压缩。
自动压缩沿用目标与研究记录的前提；前提不足或无法减小上下文时会明确暂缓。
摘要有损，可能省略历史输出和约束；需要长期保留的规则请放入 `prompts.project_file`。

在 Chat 中用 `/image PATH` 选择本地图片。可重复输入该命令；图片按输入顺序排队，下一条普通任务会将整队图片作为同一 Turn 的初始上下文。`primary_direct` 会将每张图片交给主模型，`secondary_tool` 会为每张图片单独保留观察后再请求主模型。后续图片的准备失败、取消或耗尽本轮请求预算时，已保留的图片事实不会丢失，但该任务和队列中的图片不会提交给主模型。队列只存在于当前 Chat 进程；重新启动或 `/resume` 不会自动重放图片处理。

### 3. 建立研究项目

```sh
mkdir -p ../echidna-research
cd ../echidna-research

"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`init` 会在此目录创建 `.echidna/`，保存项目事件、Session、Artifact 和 Memory。
已有完整的 `.cyber-agent/` 项目仍可原地读取，不会迁移或复制；同一项目根目录不要同时保留两个控制目录。

`doctor` 在本地检查配置，不读取环境变量密钥、不发送模型请求，也不验证密钥有效性或远端服务可达性；
进入 `chat` 提交任务后才使用所配置的模型。

在独立研究目录启动：工具使用当前用户权限执行，当前没有内置沙箱或执行确认层。

## 日常使用

查看历史会话，或者在新进程中继续最近的会话：

```sh
"$ECHIDNA_BIN" session list
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat --resume latest
```

需要限定研究领域时，在 `chat` 后加 `--domain source|web|mobile|binary` 中的一个值，
例如 `chat --domain binary`。领域指导不会自动安装分析工具。

| 交互 | 行为 |
| --- | --- |
| `/stop` 或 Ctrl+C | 请求停止当前 Turn；已执行操作不会被撤销，后台任务需要单独处理。 |
| `/paste` | 按提示输入多行任务或补充材料。 |
| `/image PATH` | 将一张本地图片追加到下一轮队列；重复输入按顺序追加。 |
| `/compact` | 在具备目标与研究记录等前提时压缩上下文；有待提交图片时不可用。 |
| `/resume` | 在可恢复停止后显式重建模型会话，再输入任务。 |
| `/exit` | 离开交互界面。 |

导出指定会话，保留对过程和材料的引用：

```sh
# 将 SESSION_ID 替换为 session list 中的实际 ID。
"$ECHIDNA_BIN" session export SESSION_ID --format markdown --output session.md
```

终端回复以 **`Echidna>`** 开头。`NO_COLOR=1` 关闭主题颜色；
管道输出不会增加品牌欢迎或主题 ANSI。开发构建额外支持 `chat --debug`，
用于查看请求内容；标准 release 不包含此选项。

## 设计取向

**模型负责提出方法，工具负责执行，原始记录负责留下依据。**

Echidna 将执行、预算、Session 和恢复留在一个内核中。模型、研究指导与方法可以继续演进，
原始材料和执行记录为后续复查提供基础。项目设计保持小而明确，方便个人理解和维护。

当前入口是交互 CLI。TUI、机器 worker 入口、外部任务队列和独立 core SDK 均处于设计阶段。
网络中断恢复仍待完善，不承诺任意断流都能自动续传。

## 下一步

| 方向 | 目标 |
| --- | --- |
| 可靠性与交互 | 改善中断诊断、恢复提示、会话与后台任务的可见性。 |
| 可复用研究 | 更方便地定位证据、交接结果，从重复实践中整理方法。 |
| 外部并行 | 先验证独立目录与多进程的实际收益，再设计明确的机器入口和协调器。 |

这些方向尚处于规划阶段，具体实施顺序由实际使用反馈决定。
外部协调独立于单个 Agent 内核演进。

## 文档

| 入口 | 内容 |
| --- | --- |
| [纯文本配置](examples/provider.toml) | 主模型、能力声明与环境变量密钥引用 |
| [单模型多模态配置](examples/provider-multimodal.toml) | 主模型直接处理文本与图像 |
| [文本 + 视觉配置](examples/provider-text-vision.toml) | 两个模型的独立服务、能力与密钥配置 |
| [MCP 配置](examples/provider-mcp.toml) | HTTP 与本地 stdio 工具、专用凭据引用和显式子进程环境 |
| [原理图原图](assets/architecture-overview.svg) | 可放大的执行与持久化流程图 |
| [更新记录](CHANGELOG.md) | 功能变化与兼容性说明 |

<details>
<summary><strong>开发与验证</strong></summary>

默认验证使用离线 Mock / Fixture / Replay 及本地 loopback：

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
```

macOS arm64 本地分发：

```sh
./scripts/package-macos-arm64.sh
./tests/distribution_package.sh dist
```

Live 测试独立、显式启用；当前仓库 Actions 关闭。
工程检查通过不代表模型在所有研究任务中都能正确完成工作。

</details>
