# 配置与接入

[返回首页](../README.md) · [English](setup.en.md)

命令按仓库根目录起步，并在同一终端中执行；进入研究目录后继续使用 `ECHIDNA_BIN`。

[1. 构建](#1-构建) · [2. 配置模型](#2-配置模型) · [环境变量与密钥](#环境变量与密钥) · [可选远程 HTTP MCP 工具](#可选远程-http-mcp-工具) · [可选本地 stdio MCP 工具](#可选本地-stdio-mcp-工具) · [ChatGPT 订阅登录](#chatgpt-订阅登录) · [Codex 兼容订阅登录](#codex-兼容订阅登录) · [单个多模态模型](#单个多模态模型) · [文本与视觉模型分开配置](#文本与视觉模型分开配置) · [3. 建立研究项目](#3-建立研究项目)

---

当前主要面向 macOS。需要 Git 和 Rust/rustup；仓库通过
[`rust-toolchain.toml`](../rust-toolchain.toml) 固定 Rust 1.93.1。
外部分析工具按任务需要自行准备。

## 1. 构建

```sh
git clone --branch main --single-branch https://github.com/F0und-icu/echidna-agent.git
cd echidna-agent
cargo build --locked --release

ECHIDNA_BIN="$PWD/target/release/echidna"
"$ECHIDNA_BIN" --version
```

后续命令在同一个终端执行，继续使用 `ECHIDNA_BIN`。

## 2. 配置模型

模板和常规开发、验收默认使用至少 **272,000 tokens** 的上下文窗口；模型明确支持
**1M** 时使用其实际声明的 1M 窗口。通过能力映射或模型目录导入设置容量，不按模型名猜测。

按使用方式选择完整模板，保存到代码仓库之外：

六份模板的字段、默认值、可选设置及复制步骤见[配置模板索引](../examples/README.md)。

| 使用方式 | 完整模板 | 图像处理 |
| --- | --- | --- |
| 纯文本模型 | [provider.toml](../examples/provider.toml) | `disabled`，不接收图像 |
| 单个多模态模型 | [provider-multimodal.toml](../examples/provider-multimodal.toml) | `primary_direct`，主模型直接接收图像 |
| 文本主模型 + 独立视觉模型 | [provider-text-vision.toml](../examples/provider-text-vision.toml) | `secondary_tool`，视觉模型分析图像，主模型使用其观察结果继续研究 |
| 文本模型 + 远程 MCP 工具 | [provider-mcp.toml](../examples/provider-mcp.toml) | `chat` 启动时发现延迟暴露的远程工具 |

下面以纯文本模板为例；需要图像能力时，将复制命令的源文件换成对应模板：

```sh
mkdir -p "$HOME/.config/echidna"
cp -n examples/provider.toml "$HOME/.config/echidna/provider.toml"
chmod 600 "$HOME/.config/echidna/provider.toml"
```

编辑该文件，填写服务地址和模型，并按服务实际能力核对窗口和各项能力声明。
模板中的地址和模型名是示例，窗口默认272K；按服务实际能力调整，`cp -n` 保留已有文件。
这里的路径由命令显式传入，程序不会自动发现它。

当前接入 **HTTP/SSE + Responses** 协议。Chat Completions 接口不适用。
分开配置时，主模型和视觉模型的服务都需要支持此协议。

### 环境变量与密钥

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

### 可选远程 HTTP MCP 工具

使用 [MCP 模板](../examples/provider-mcp.toml)，在 `mcp_servers` 下声明一个或多个远程
Streamable HTTP 工具服务器。每个服务器需要绝对 HTTP(S) `url`，以及
`protocol_version = "2025-11-25"` 或 `protocol_version = "2026-07-28"` 之一。
可选 `timeout_ms` 默认为 60000，限定每次工具调用。HTTP 和 stdio 均可单独配置
`startup_timeout_ms`，限定该服务器启动、初始化和目录发现的总时间；省略时沿用
`timeout_ms`。需要短调用超时但启动较慢时，可设置例如 `timeout_ms = 1000`、
`startup_timeout_ms = 10000`。启动失败会显示服务器名和具体阶段，不跳过或重试。

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

### 可选本地 stdio MCP 工具

同一个 [MCP 模板](../examples/provider-mcp.toml) 也支持 Unix stdio 服务器，每项在 `url`
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

### ChatGPT 订阅登录

使用 [订阅模板](../examples/provider-chatgpt-subscription.toml)，在仓库外保存自己的配置。
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
模板默认272K，明确支持1M时填写或导入实际窗口。HTTP 订阅路径要求 `incremental_input = false`，
不能设置 `max_output_tokens`。登录成功和离线 doctor 不代表账户已获模型使用资格。

```sh
echidna --config /absolute/path/subscription.toml auth logout --provider chatgpt
```

访问令牌在调用前按需刷新；注销会尝试撤销授权并清除本地令牌，结果会说明远端撤销
是否确认。请将认证目录保存在仓库和研究项目之外，目录/文件仅允许当前用户访问。
订阅额度与服务端授权受 [官方预览限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)
约束，失败不会自动切换为付费 API 或重跑工具。其它供应商的非 API Key 订阅尚未接入。

### Codex 兼容订阅登录

使用 [Codex 兼容模板](../examples/provider-codex-subscription.toml)，显式选择
`codex_subscription` 并保留 `https://chatgpt.com/backend-api/codex` 地址。
它与上述 SIWC 以及两种 API Key 配置互斥。请使用独立的私有 `auth_store`，
不要与 SIWC 共用；原有 API Key、Qwen、DeepSeek 配置无需修改。

如需声明目录兼容版本，可在 `codex_subscription` 内设置
`catalog_client_version = "major.minor.patch"`。缺省值保持现有 `0.144.0`；
显式值只会同时改变目录请求的 `client_version` 查询参数和 `version` 请求头，
不是已安装 Codex 版本声明，也不保证返回新模型 ID 或账户访问资格。
当前模板显式使用曾在目录对照中验证过的 `0.162.0`，并写出
`supports_reasoning_summary_parameter = true`；不是自动探测或“永远最新”的版本。
目录仍由服务端按账户和兼容版本决定，不能通过配置保证拿到所有模型。

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

`auth models` / `auth select-model` 也支持 API Key 或 `credential_ref` 的标准
Responses 服务（包括 DeepSeek）：从所选 Provider 的 `base_url` 请求 `/models`，
将下面的配置路径和 `--provider` 换成自己的值即可。登录、状态、刷新、退出仍仅用于订阅。
可用交互选择器查看目录、选择模型与推理强度，并保存到当前配置：

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex
```

若目录声明了模型能力，选型时加 `--apply-capabilities` 同步实际提供的窗口等信息：

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex --apply-capabilities
```

只换模型 ID 不会自动修改 `context_window`；模板默认按272,000规划，支持1M时需明确配置或导入。
没有可导入窗口时，按该服务明确给出的容量填写，不要按模型名猜。

也可指定目录返回的模型和该模型支持的强度：

```sh
echidna --config /absolute/path/codex.toml auth select-model --provider codex \
  --model gpt-6-astra --reasoning-effort xhigh
```

执行强度支持目录声明的有效字符串，包括未来新增的值；交互选择以当前模型目录为准。
API Key 目录未声明强度时，可明确输入 `--reasoning-effort` 或在选择器中输入强度，
程序会提示支持情况未知，服务端仍可能拒绝该值；已声明强度时只接受目录内的值。
直接配置的 `reasoning_effort` 接受 1–64 个 ASCII 字母、数字、下划线或连字符。
省略强度时使用服务端默认行为；显式 `none` 是不同的请求值，不等于省略。
保存会为该 Provider 复制独立能力映射，保留其他模型、注释和凭据。
配置锁协调同时运行的 Echidna 选择命令；保存期间请勿用其他编辑器同时修改该配置。
加上 `--apply-capabilities` 可同时导入该模型目录中存在的默认上下文窗口、文本/图像
输入能力及摘要参数支持声明。缺失字段或未知窗口（包括 `null`）保留当前配置；`max_context_window` 仅展示为
元数据，不会代替默认窗口。导入仍需通过现有配置和输出预算校验，失败不覆盖原文件。
不加此选项保持原有选择行为；工具等其余能力仍须显式核实，不按模型名推断。
仅含模型 ID 的 API Key 目录保留原能力映射，使用 `--apply-capabilities` 会明确失败而不保存。
API Key 目录可导入实际声明的 `context_window`、`input_modalities`；服务端的
`max_output_tokens` 和其他协议能力不会变成当前配置的输出预算或 Responses 能力。
`doctor` 可离线查看生效的强度，下一次启动 `chat` 使用保存的设置。
该兼容链路依据固定版本 Codex 源码实现，实际账号和服务端支持须通过实测确认。
失败不会切换模型、付费 API 或自动重跑工具。

Codex 链路默认请求服务端可用的推理摘要，在 stderr 上标注 `reasoning summary`，包括 release
构建。这是提供方返回的摘要，不是完整内部推理；摘要为空时不会补造内容。摘要不进入
对话正文，也不替代加密续接。推理强度与摘要展示独立，设置 `high` 不保证返回摘要。
若服务端不接受摘要参数，可在 `codex_subscription` 中显式设置
`supports_reasoning_summary_parameter = false`；省略或设为 `true` 保持默认请求。

### 单个多模态模型

使用 [多模态模板](../examples/provider-multimodal.toml)，由同一个主模型处理文本、工具调用和图像。
其 `[capability_mappings.primary-v1]` 声明 `image_input = true`，组合配置为：

```toml
[agent_profiles.default]
primary = "primary"
vision_mode = "primary_direct"
generation = 1
```

只需设置主模型密钥，无需额外配置视觉 Provider。

### 文本与视觉模型分开配置

使用 [双模型模板](../examples/provider-text-vision.toml)，分别填写 `[providers.primary]`
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
默认为 `32`，是每轮主模型与视觉模型共享的请求次数上限。显式设为 `0` 可取消此上限；
`1..=10000` 保持有限额度。不限请求仍记录真实尝试次数，且不改变上下文容量、取消、
超时和单次请求的重试上限。

上下文额度可在同一配置中按比例分配。`_bps` 使用基点：**100 = 1%，10000 = 100%**。
未填写的字段采用默认策略。输出/工具预留按声明窗口计算，普通内容额度按扣除预留后的
可用输入计算；诊断任务/交接摘录的分母则是诊断池。显式输出要求仍优先。
内容字节额度沿用当前估算换算，完整输入须通过实际序列化预检，不代表服务端精确分词。

```toml
[runtime.context_budget]
output_reserve_bps = 1250       # 窗口的12.5%
tool_loop_reserve_bps = 1250    # 窗口的12.5%
pressure_bps = 8000            # 可用输入的80%时评估自动压缩
target_bps = 5000              # 压缩后优先目标50%，保留必需数据
summary_bps = 200              # v6 摘要 Narrative JSON 额度2%
diagnostic_bps = 200           # RSI诊断独立额度2%，不与摘要调参联动
diagnostic_task_bps = 313      # 任务摘录占诊断额度3.13%，可按任务调整
diagnostic_handoff_bps = 313   # 上一交接摘录占诊断额度3.13%
tool_output_bps = 3200         # 每个内置工具的预览额度32%
core_memory_bps = 1600         # Core工具内容额度16%
state_section_bps = 10         # 每个状态节0.1%
continuation_field_bps = 40
continuation_goal_part_bps = 16
readback_default_bps = 100     # 精确回读默认1%
readback_max_bps = 400         # 精确回读最大4%
compaction_reference_items = 64  # 每类引用数量上限，单位：项
max_core_file_bytes = 524288     # Core存储上限，单位：字节，可显式调整
# max_content_bytes = 1048576    # 可选的绝对内容上限；未设置时仅按比例
```

自动压缩仍由 `[runtime].auto_compact = true` 开启。百分比是分配策略；原任务、图片和
当前完整工具批次不会为达到软目标被静默删掉。超预算的合法会话可在显式恢复后执行
`/context`、`/compact` 或 `/exit`；恢复和压缩不会自动执行旧任务或重放工具。
无工具的压缩采样只释放工具循环预留，仍保留输出/推理空间，并计入共享请求预算。
v6 的机器来源与保护引用单独验真，完整模型输入仍须通过硬预算预检；历史摘要的字节额度含义保持。

在 Chat 中用 `/image PATH` 选择本地图片。可重复输入该命令；图片按输入顺序排队，下一条普通任务会将整队图片作为同一 Turn 的初始上下文。`primary_direct` 会将每张图片交给主模型，`secondary_tool` 会为每张图片单独保留观察后再请求主模型。后续图片的准备失败、取消或耗尽本轮请求预算时，已保留的图片事实不会丢失，但该任务和队列中的图片不会提交给主模型。队列只存在于当前 Chat 进程；重新启动或 `/resume` 不会自动重放图片处理。

## 3. 建立研究项目

```sh
mkdir -p ../echidna-research
cd ../echidna-research

"$ECHIDNA_BIN" init
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" doctor
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat
```

`doctor` 在本地检查配置，不读取环境变量密钥、不发送模型请求，也不验证密钥有效性或远端服务可达性；
进入 `chat` 提交任务后才使用所配置的模型。

在独立研究目录启动：工具使用当前用户权限执行，当前没有内置沙箱或执行确认层。
