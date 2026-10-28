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

## 外接 Agent 定义（独立分支）

`codex/agent-definition-v1` 将人格、工作内容和方法外置为本地文件，保留同一个
执行循环。该分支独立维护，不合并主分支。默认仍为研究 Agent；也可使用
[`examples/agents/technical-writing`](examples/agents/technical-writing) 切换为技术写作 Agent。

```sh
echidna agent init ./my-agent --id my-writer --from technical-writing
# 编辑 my-agent/persona.md、work.md 和 methods/ 下的 Markdown。
echidna --config /path/to/provider.toml agent check ./my-agent
echidna --config /path/to/provider.toml agent inspect ./my-agent
# 编写精确的 MCP Tool 名称时，显式允许一次目录发现。
echidna --config /path/to/provider.toml agent inspect ./my-agent --discover-mcp
echidna --config /path/to/provider.toml chat --agent ./my-agent
```

`agent init --from builtin-research` 导出完整研究模板。`check` 离线验证资源、
工具与静态上下文预算；任务历史和当前事实的预算在运行时继续检查。
`inspect` 展示最终指令与工具，请留意其中可能包含你自己的指导文件内容。

`check` 和默认 `inspect` 保持离线：不会连接 MCP 服务或读取 MCP 凭据。编写外接
Tool 声明时，先对尚未声明该远程 Tool 的定义运行 `agent inspect --discover-mcp`。
该命令只发现一次已配置的 MCP 目录，即使工具当前隐藏也会输出
`discovered_mcp_tools`，随后继续进行原有静态绑定和预算验证；它不会发起模型请求或
持久化目录。将输出中的精确本地名称写入 `[tools]`，例如
`mcp__fixture__0123456789abcdef0123 = "deferred"`；不支持通配符或默认规则。

定义目录的 `agent.toml` 使用 `schema_version = 1`：

| 配置 | 含义 |
| --- | --- |
| `id` / `name` | 稳定身份与显示名称；修改文字内容不会改变 Agent 记忆身份。 |
| `instructions` | `persona_file`、`work_file`、有序 `method_files`；路径相对定义目录。 |
| `context.personal` / `project` | `configured` 加载 Provider 配置指定的指导文件；`none` 不加载。 |
| `context.memory` | `none`、`project` 或 `agent`；Agent 记忆按项目和稳定 `id` 隔离。 |
| `tools` | 已知内置工具或精确发现的 MCP 名称的 `direct`、`deferred`、`hidden`；省略即 Hidden，延迟工具要求直接提供 `tool_search`。 |
| 可选 `research` | 研究领域、阶段及完整的外部指导文件映射；省略后没有研究状态或默认研究事件。 |

研究定义的 Direct/Deferred 是基础可见性，现有 Domain/Phase 规则可进一步调整；
Hidden 始终保持隐藏。工具可见性用于控制上下文，不是权限或沙箱。
通用 Agent 的图片、HTTP 材料和产物属于当前 Session，不需要创建虚假研究 Target。

运行时会向模型明确提供当前 Agent ID 和 Memory 作用域。Memory 逻辑路径
相对于该作用域；读取不存在的文档返回 content/hash 为 null，新建时用
expected_hash:null，更新时使用 memory_read 返回的当前哈希。
memory_read、memory_write、inspect_project 的 v2 契约由运行时填写本次操作
时间；调用参数不再接受 occurred_at/observed_at。历史会话与来源时间保持原样。

在 chat 输入 `/memory` 查看实际保存目录、作用域、`core.md` 是否存在及记忆工具
配置。项目记忆位于 `.echidna/memory/`，Agent 记忆位于
`.echidna/memory/agents/<agent-id>/`；旧项目继续使用原来的 `.cyber-agent/`。
首次成功的 `memory_write` 才创建 Markdown，普通回答或压缩不会写入记忆。
启用记忆的 Agent 会收到写入时机和读写方法的内置指引：保留有复用价值的经验或
明确要求记住的内容；`core.md` 会自动进入后续上下文，详细笔记需按需读取。
没有后台模型提取任务，也不保证每轮都会产生记忆。
新版固定指导要求新写入的记忆叙述使用英文，精确引文、代码、路径和标识保留原样；不会自动翻译旧记忆。

会话内使用 `/agent` 查看身份和定义摘要；`/agent reload [目录或 agent.toml]`
显式重载当前身份的指令，保留 History 并重建 Provider 会话。`/agent use <路径>`
打开新的 Session，旧会话仍可恢复。切换要求没有未配对调用、运行中或停止中的
Job/PTY、待提交图片；`/image cancel` 可撤下待提交图片而保留原始产物。

会话保存实际使用的完整指令。修改或删除源文件不会改变已有会话；Resume 使用保存的
内容，reload 才读取新内容。reload 不允许改变 `id`、记忆作用域、研究契约或 RSI 开关，
这类变化使用 `use`。两个命令都不会自动请求模型、继续旧任务或重放工具。
新会话选择顺序为 `chat --agent`、Provider 配置顶层 `agent_definition`、内置研究定义；
配置中的路径相对配置文件。模型与凭据配置保持独立。旧二进制不能读取新定义事件。

### 可选 RSI 自我迭代

`[rsi]` 可另设 `improvement = true`；缺省为 `false`，原被动 RSI 行为和旧定义身份保持不变。
主动模式在正常完成时，或长任务每16个完整耐久工具批次后，检查尚未评估的真实失败、
同一已校验输入的重复命令或以 `[rsi-feedback]` 开头的明确长期纠正。
要求历史配对、无后台任务/PTY及待处理图片。诊断材料按当前窗口限制大小，并优先最近8个摩擦来源。
没有新信号或共享预算不足时不启动。完成边界需至少3次余量，长任务检查点需至少4次。
一次改进包含诊断、旧/新对照；同轮应用另需一次无工具交接请求，每个逻辑采样最多尝试3次，最多12次实际请求，
全部计入原轮次预算，额度不足会提前终止。`no_change` 只消耗诊断阶段实际发生的尝试。

诊断仅提出人格或方法中一个范围的改动，方法优先。工具代码仍走原有普通脚本生成、
实际执行与 RSI 发布链。探针仅保留当前轮次最后已提交计划里实际曝光的 `run_command`；
本轮由 `tool_search` 激活的命令可继续使用，Hidden 不被提升，激活不会带入下一轮。
主动模式的 `run_command` / `run_commands` 可用
`code_inputs = ["script.py", "input.csv"]` 让宿主计算并留存执行前后 BLAKE3；
显式 `verified_inputs` 的预期摘要仍受校验，两者合计最多 8 个文件。
宿主只从当前轮次成功命令的完整原始输出建立隐藏的 `reference_match` 参考，
在相同输入的两份副本上各执行一个新命令；只有旧结果不匹配而新结果匹配时才允许试用。
持平、回退、输入变更、空参考或缺少完整捕获均不自动采纳。
长任务中有实际局部增益的候选经成功交接、完整工具批次预检与耐久定义切换后，即用于同一Turn的下一次请求。
保留原任务、预算、工具激活和已执行结果；交接失败保持旧指令及待处理候选。
这只说明该次局部输出对照，不能证明一般正确性、性能或跨任务收益。

采纳后的经验按显式任务子串 cue 选入上下文，携带原对照和回执引用，始终是派生指导。
它不代替当前验证。新活动事件与普通聊天历史分开保存在同一 Session；
旧程序不能恢复含这些事件的新会话。未知执行与持久化失败保持致命，恢复不会重跑探针。

给外接定义添加以下配置，或从 [`examples/agents/self-improving`](examples/agents/self-improving)
开始。原定义保持默认关闭；按需要在 `[tools]` 显式提供文件与命令工具，
模型只能使用已曝光工具完成对应修改和测试。

```toml
[rsi]
persona = true
methods = true
fixed_rules_file = "fixed.md" # 可选；操作员固定规则，不属于 AI 可修改内容
```

人格可以替换或精简；方法也可以删除。工作范围、Personal/Project、工具曝光、记忆作用域、
Research、模型配置及 RSI 协议不属于可修改范围。主机单独注入只读协议和固定规则，
给出当前 Agent/Session、父版本和精确候选路径。模型用现有文件工具写入严格 JSON，
无需逐项确认；被动模式不增加反思请求。FullAccess 仍然存在，这里的只读是更新入口的
所有权约定，不是文件系统保护或模型服从保证。

当前 Turn 内所有请求固定使用原指令。正常完成、历史持久化且调用结果配对、没有活跃
Job/PTY 或待提交图片时，每轮最多发布一批；下一次请求使用新版本和新的 ProviderSession。
取消、停止和执行效果未知时保留候选；既有显式恢复要求不变。错误候选会显示拒绝原因，
保留文件以便修正，不会悄悄覆盖原人格文件。

版本保存在项目 `.echidna/evolution/<agent-id>/revisions/<digest>.json`，
`active.json` 选择新 Session 的缺省版本。已有 Session Resume 使用其保存的完整绑定；
`/agent` 可查看当前版本。落后于已发布版本的旧 Session 可以显式 `/agent reload` 对齐。
Personal/Project 仍由每个新 Session 显式快照，不随人格版本覆盖旧会话。
新项目的历史、记忆、产物和缓存统一保存在 `.echidna/`。已有 `.cyber-agent/`
项目继续原地使用，历史和脚本路径不会自动搬迁；同一项目根同时存在这两个目录时
程序会明确拒绝，请先确认两边数据来源，避免形成两套会话和锁。
手动修改源定义后会提示源分支冲突，执行以下离线命令明确选择新源并保留旧版本：

```sh
echidna --config /path/to/provider.toml agent lineage ./my-agent
# 定义含显式远端 MCP 名称时，仅通过此旗标允许一次目录发现。
echidna --config /path/to/provider.toml agent lineage ./my-agent --discover-mcp
```

回滚也通过候选文件完成：保留协议要求的身份、父版本、reason/basis 字段，添加
`"rollback_to":"旧版本 digest"`，不要同时包含人格、方法或工具修改。回滚生成新版本，
历史请求与旧版本不变。版本文件、Session 绑定事件、发布指针和候选消耗不是跨存储事务；
如果绑定已提交但发布指针失败，错误会终止进程。显式 Resume 该 Session 后运行
`/rsi reconcile`，只有指针仍指向其父版本时才补齐；它不发请求、不重跑工具、不重建 Provider。

模型可在任务中写脚本并直接运行，然后发布小型脚本索引。启用 RSI 的绑定提供 `run_command` 的可选
`verified_inputs=[{path,blake3}]` 对项目相对代码文件在执行前后核验字节，成功结果保留
实际核验值；不填写时参数和结果不增加该字段。发布要求真实成功的调用、配对结果和
Receipt，当前代码仍匹配，并在项目根目录测试。多文件脚本按完整文件清单复制到同一个
版本目录，保持相对目录；索引中的 argv 只替换完整匹配的路径参数，后续使用该 argv。
包内依赖应以脚本自身位置解析；运行时不会重写脚本内容或任意路径字符串。
Unix 发布副本按源文件是否可执行使用私有0700/0600模式，执行属性参与包身份；直接入口
发布时仍须可执行。更改执行属性会生成不同包路径，原包的模式不变，也不自动选择解释器。
非Unix平台明确拒绝脚本发布，人格和方法更新保持可用；不推测其他系统的执行权限语义。
完整版本按实际JSON编码限制为1MiB，包含逐轮累积索引与转义；超限候选保留，旧版本继续可用。
依赖说明仅作记录，不自动安装。此证据说明一次调用前后的代码身份和退出结果，
不证明进程内部加载字节、测试覆盖率或语义改进；实际任务反馈决定保留、修改或回滚。

### 长上下文容量

显式 `capability_mapping.context_window` 决定本地容量，运行时不按模型名猜测。
输出预留为 `min(W/8, 65536)`，工具循环预留为 `min(W/8, 131072)`；已有显式输出
上限仍可提高前者。这些预留消耗输入空间，不是服务端输出上限或额外窗口。

工具输出、Core Memory、摘要和状态片段按 `clamp(floor(W/131072), 1, 8)` 倍扩展。
例如 272000 窗口使用 256KiB 工具输出、128KiB Core 和16KiB摘要；输入容量为204000。
1048576 窗口使用1MiB工具输出、512KiB Core 和64KiB摘要；输入容量为851968。
小窗口保留原有128KiB工具输出、64KiB Core、8KiB摘要和384字节状态片段。
命令 stdout/stderr 共享工具预览容量；完整原始捕获、传输和 MCP 边界保持各自原有上限。

大摘要保存为含自身字节上限的 v4；历史v1至v3不变。切换到小窗口不会使已保存的大摘要
或Core文档失效，后续请求仍须通过当前上下文预算。Core作为完整来源交给现有规划器，
是否全部发送由选择与截断记录说明；较小模型的Memory工具读取或写入超限内容会明确失败。

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

`auth models` / `auth select-model` 也支持 API Key 或 `credential_ref` 的标准
Responses 服务（包括 DeepSeek）：从所选 Provider 的 `base_url` 请求 `/models`，
将下面的配置路径和 `--provider` 换成自己的值即可。登录、状态、刷新、退出仍仅用于订阅。
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
默认为 `32`，是每轮主模型与视觉模型共享的请求次数上限。显式设为 `0` 可取消此上限；
`1..=10000` 保持有限额度。不限请求仍记录真实尝试次数，且不改变上下文容量、取消、
超时和单次请求的重试上限。

上下文额度可在同一配置中按比例分配。`_bps` 使用基点：**100 = 1%，10000 = 100%**。
未填写的字段采用默认策略。输出/工具预留按声明窗口计算，其他内容额度按扣除预留后的
可用输入计算；显式输出要求仍优先。额度按最终编码预检，不代表服务端精确分词。

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

### 3. 建立研究项目

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
| `/compact` | 请求当前主模型生成无工具的工作交接，再验证并压缩上下文；使用独立命令请求预算。有待提交图片时不可用。 |
| `/resume` | 在可恢复停止后显式重建模型会话，再输入任务。 |
| `/exit` | 离开交互界面。 |

可在模型配置中开启自动压缩，缺省关闭：

```toml
[runtime]
auto_compact = true
# 可选；不填写时继承主模型配置，仅影响独立压缩请求。
# compaction_reasoning_effort = "high"
```

显式设置压缩推理强度时，压缩仍使用相同的主模型、端点和凭据；主任务和视觉请求不变。
该设置需要服务端支持，服务端拒绝会直接显示，不会自动降级。

每次任务正常完成并处理适用的 RSI 边界后，或完整工具批次的结果已持久化、准备继续采样时，
检查上下文压力。工作集阈值为输入预算的 80% 与 65,536 估算 tokens 中的较小值；
工具循环压缩目标为该阈值的一半，保留当前任务原文和图片、同一轮请求预算及工具激活。
工作集控制不改变模型支持的上下文窗口。每次可用压缩通过配置的主模型额外生成一次无工具摘要，
自动压缩与原任务共用请求预算；连接重试也计入预算。保留最近完整工具批次和最新收据/历史来源。
Resume 遇到追加的 Research 记录时，离线更新引用并保留此前派生交接；此刷新不发模型请求。
成功后下一次请求为 `CompactionReset Full`。人格、指令快照、作用域记忆及已发现 MCP
目录保持原有契约。研究前提不足、无法缩减，或摘要无效、超限、取消、请求失败时会显示推迟原因，
保留原上下文与模型续接，不静默换用旧助手正文。摘要仍需真实任务另行验证效果。
现代摘要发送给模型时省略机器引用列表，完整耐久事实、来源信息、叙述及 Research 脉络仍会保留。
摘要有损且不替代稳定规则。取消、失败和已停止的回合不自动恢复、重发或丢弃推理；
当前任务或固定指令过大、缺少压缩前提时仍可能超出预算，这时继续使用显式 `/resume` 和 `/compact`。

成功完整样本若返回用量，终端显示 Provider 的 input/output totals，以及可选 cached-input/
reasoning-output 子集，并绑定实际请求审计保存。子集已经包含在 totals 中；缺失保持 unknown。
本地 Context 估算与请求准入次数不是 Provider 用量。写文件的模型回显只保留实际路径、
哈希和大小；完整 diff 仍保存在执行回执中，避免在后续每次请求里重复发送。

模型连接建立失败或完整响应前发生传输中断时，默认对同一请求最多重试 2 次，
分别等待 500ms 和 1000ms；`/stop` 和 Ctrl+C 可取消等待。每次尝试都占用原请求额度，
并单独记录请求审计。请求内容、凭据和续接状态保持不变；重试会从头生成当前响应。
已经显示的失败响应片段会被明确标记作废，只有完整成功的样本才写入会话并执行其工具。
已完成的工具不会重跑。HTTP 拒绝、协议或凭据错误不自动重试；重试耗尽后保留检查点，
按停止提示使用 `/resume` 再输入新任务。RSI 诊断和局部对照同样共用原预算，
采样重试不重开 arm、不重跑已经完成的对照命令，也不自动采纳失败候选。
同一 RSI 作用域发生过多次请求审计的新会话须使用新版恢复；旧版程序会拒绝读取，
不应切回旧程序继续该会话。
工具参数JSON无效仍在整个样本执行前拒绝；每个连续失败链最多重新生成2次。
完整样本耐久提交后才归零，取消、传输失败及无效兄弟调用不归零；不会重放已完成工具。

导出指定会话，保留对过程和材料的引用：

```sh
# 将 SESSION_ID 替换为 session list 中的实际 ID。
"$ECHIDNA_BIN" session export SESSION_ID --format markdown --output session.md
```

终端回复以 **`Echidna>`** 开头。`NO_COLOR=1` 关闭主题颜色；
管道输出不会增加品牌欢迎或主题 ANSI。开发构建额外支持 `chat --debug`，
用于查看请求内容；标准 release 不包含此选项。

所有构建均支持显式保存失败请求的脱敏诊断快照：

```sh
"$ECHIDNA_BIN" --config "$HOME/.config/echidna/provider.toml" chat --debug-failures ../debug-failures
```

目录相对于 chat 的实际工作目录解析；建议放在项目目录外。默认不启用，成功或取消的
请求不保存；每次失败的物理请求（包括后续重试成功前的失败）独立保存一个文件。
Unix上新目录权限为0700，文件为0600且不覆盖；保存成功或失败均在终端报告，原请求错误和
重试行为保持不变。快照保留提示词、输入和工具定义，移除认证字段、当前凭据/账户值、
opaque内容、图片data及URL认证/查询/片段。它是脱敏诊断，不能视为原始网络包或自动
重放材料；业务文本中的其他未知秘密仍可能保留，请自行决定是否分享。

## 设计取向

**模型负责提出方法，工具负责执行，原始记录负责留下依据。**

Echidna 将执行、预算、Session 和恢复留在一个内核中。模型、研究指导与方法可以继续演进，
原始材料和执行记录为后续复查提供基础。项目设计保持小而明确，方便个人理解和维护。

当前入口是交互 CLI。TUI、机器 worker 入口、外部任务队列和独立 core SDK 均处于设计阶段。
网络恢复限于上述未完成模型请求的有限重试，工具执行保持原有结果与恢复规则。

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
