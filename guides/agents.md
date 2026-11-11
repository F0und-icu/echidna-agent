# Agent 定义

[返回首页](../README.md) · [English](agents.en.md)

[记忆与作用域](#记忆与作用域) · [切换、重载与恢复](#切换重载与恢复) · [可选 RSI 自我迭代](#可选-rsi-自我迭代)

---

`codex/agent-definition-v1` 将人格、工作内容和方法外置为本地文件，保留同一个
执行循环。该分支独立维护，不合并主分支。默认仍为研究 Agent；也可使用
[`examples/agents/technical-writing`](../examples/agents/technical-writing) 切换为技术写作 Agent。

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
| `tools` | 已知内置工具或精确发现的 MCP 名称的 `direct`、`deferred`、`hidden`；省略即 Hidden，但 Chat 的已绑定 `context_read` 默认 Direct。显式声明优先，延迟工具要求直接提供 `tool_search`。 |
| 可选 `research` | 研究领域、阶段及完整的外部指导文件映射；省略后没有研究状态或默认研究事件。 |

研究定义的 Direct/Deferred 是基础可见性，现有 Domain/Phase 规则可进一步调整；
Hidden 始终保持隐藏。工具可见性用于控制上下文，不是权限或沙箱。
通用 Agent 的图片、HTTP 材料和产物属于当前 Session，不需要创建虚假研究 Target。

## 记忆与作用域

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
明确要求记住的内容；`core.md` 按当前任务、来源范围和修订选用，明确的全局约束保留，
详细笔记按需读取。选择上下文不会重写记忆文件或裁剪已绑定的人格、工作和方法指令。
没有后台模型提取任务，也不保证每轮都会产生记忆。
新版固定指导要求新写入的记忆叙述使用英文，精确引文、代码、路径和标识保留原样；不会自动翻译旧记忆。

## 切换、重载与恢复

会话内使用 `/agent` 查看身份和定义摘要；`/agent reload [目录或 agent.toml]`
显式重载当前身份的指令，保留 History 并重建 Provider 会话。`/agent use <路径>`
打开新的 Session，旧会话仍可恢复。切换要求没有未配对调用、运行中或停止中的
Job/PTY、待提交图片；`/image cancel` 可撤下待提交图片而保留原始产物。

会话保存实际使用的完整指令。修改或删除源文件不会改变已有会话；Resume 使用保存的
内容，reload 才读取新内容。reload 不允许改变 `id`、记忆作用域、研究契约或 RSI 开关，
这类变化使用 `use`。两个命令都不会自动请求模型、继续旧任务或重放工具。
新会话选择顺序为 `chat --agent`、Provider 配置顶层 `agent_definition`、内置研究定义；
配置中的路径相对配置文件。模型与凭据配置保持独立。旧二进制不能读取新定义事件。

## 可选 RSI 自我迭代

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

给外接定义添加以下配置，或从 [`examples/agents/self-improving`](../examples/agents/self-improving)
开始。原定义保持默认关闭；按需要在 `[tools]` 显式提供文件与命令工具，
模型只能使用已曝光工具完成对应修改和测试。

```toml
[rsi]
persona = true
methods = true
improvement = true           # 可选：启用上述有额外请求开销的诊断/局部对照
fixed_rules_file = "fixed.md" # 可选；操作员固定规则，不属于 AI 可修改内容
```

人格可以替换或精简；方法也可以删除。工作范围、Personal/Project、工具曝光、记忆作用域、
Research、模型配置及 RSI 协议不属于可修改范围。主机单独注入只读协议和固定规则，
给出当前 Agent/Session、父版本和精确候选路径。模型用现有文件工具写入严格 JSON，
无需逐项确认；被动模式不增加反思请求。FullAccess 仍然存在，这里的只读是更新入口的
所有权约定，不是文件系统保护或模型服从保证。

被动 RSI 在当前 Turn 内保持原指令。正常完成、历史持久化且调用结果配对、没有活跃
Job/PTY 或待提交图片时，每轮最多发布一批；下一次请求使用新版本和新的 ProviderSession。
显式开启 `improvement` 后，长任务还可在前述完整工具批次边界完成诊断、局部对照与
耐久交接，再在同一 Turn 中使用已接受的新指令。每次采样的指令和工具计划仍固定。
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
