# CTF 长任务问题清单与方案讨论

- 更新日期：2026-09-12。
- 状态：操作者已授权连续完成修复、独立审计、限额 Live、提交与普通合并；未封存的问题不能提前关闭，准确进度以 STATUS 顶部为准。
- 本文件是后续方案讨论的问题索引，不是实施授权、冻结架构或完成验收。
- 修复计划：[CTF_LONG_TASK_REPAIR_PLAN.md](CTF_LONG_TASK_REPAIR_PLAN.md)。R1 已经 PR #70 远端合并，R2a 独立审计通过、完整 Gate 重跑待完成；其余包按序推进。Actions、登录访问、目标答案及越额 Live 仍排除。
- 测试目录：`/tmp/cyber-agent-ctf-live.VBUX7a`。
- 分轮详细证据与历史指纹：`/tmp/cyber-agent-ctf-live.VBUX7a-issues.md`。该文件和原始日志留在 Git 外；同名日志已被后续运行覆盖，不得混用不同轮次的 hash。
- 当前证据可用性（2026-09-12 计划检查）：上述目录及旁路 issues 文件均已不存在。以下保留的是此前分析结论，不是本轮对原始日志的重新核验；实现前需补合成最小回归，新的 Live 日志分轮保留。
- 测试不读取题目的 exp/wp/writeup/flag 或现成解法。原始目标材料、会话和请求内容不复制到本仓库。

## 当前边界

只实施 STATUS 当前 Scope Lock。Live 与交付按顶部连续授权的限额和验收顺序执行；Docker 新资源范围仍需明确，不接管未知容器。个人知识库/检索增补仅为计划，不实施。

此前已实现的 debug 工具定义隐藏改动保留在本地，未撤销、未提交。其 13 项唯一离线用例通过是已有结果，不是本次重新测试；未进行独立审计、完整 Slice Gate 或 release 重建。其他问题均未宣称修复。

## 问题与证据

### CTF-01：Resume 后开放 ToolCall 跨用户消息补齐（P1）

已证实修复前基线 History Normalizer 把 synthetic cancelled Result 统一追加到尾部。历史 Session `chat-1789135310-323861000-13638-1` 的 RequestAudit sequence 85/88 中，目标 Call 位于 content index 24，Result 位于 50/51，中间有后续 user。用户观察到远端 `No tool output found for tool call` 400；原 HTTP 日志已删除，本轮未重新请求。

缺少 `user -> open call -> later user` 的跨轮次回归及实际交付断言。后续方案应保持唯一 Normalizer、真实结果优先和中断副作用未知，不能制造执行 Receipt 或自动重跑。

修复进度：`CTF-R1` 已用永久回归重现上述位置错误，并在唯一 History owner 改为下一 User Text/Image 前补 unknown；迟到真实 Result 明确拒绝。独立审计、完整 Gate/Lean 通过并经 PR #70 合并。离线实现已封存；本轮 Live 尚未执行。

### CTF-02：续接拒绝与“上下文超限”混淆（P1）

Session `chat-1789138136-423411000-53629-1` 有 13 个 RequestAudit、20 个真实工具收据；最后结果已落盘，没有第 14 次请求。终止来自本地 `ProviderContinuationUnavailable`，不是新的 HTTP 400。

配置窗口为 1048576；末次已发送请求的审计组件总大小 178464 bytes，不是 token usage，不能据此声称耗尽 1M。最后请求发出后，project.md 在回复完成前被编辑；代码的 PromptChanged、累积状态差分与 exact reasoning replay 限制支持这一组合触发方向，但未用回归/分支跟踪最终证明。

R2a 离线修复已通过独立审计、完整 Gate/Lean：区分 Memory 替换、基线/History/投影不兼容和实际交付候选超预算；预算字段来自既有 Planner，不是 usage。五组夹具证明 Prompt-only/普通状态差分允许，Memory 与组合 Prompt+Project 拒绝，不外推原始已删除日志的唯一根因。可恢复交互属于 R2b，尚未实现；没有静默丢弃 reasoning 或自动重跑工具。

### CTF-03：环境已探测，但未验证任务所需工具（P2）

模型确实收到了 macos、x86_64 环境信息。当前 inspect_environment 的固定 toolchain 主要是 cargo/java/node/python3/rustc，不代表 readelf、GNU nm、timeout、Docker 或容器内工具可用。旧测试 Project Prompt 又要求使用主机缺失的 timeout，形成错误引导。

待讨论：按需确认执行位置、程序路径/版本和必要能力，缓存已确认观察并在环境变化时刷新。避免每个 Step 无差别重跑所有探测；不建设通用环境注册框架。

### CTF-04：macOS/GNU 差异与 cwd 误诊（P2）

R4b 离线验收完成：独立审计/有限复审、完整 Gate476通过/2 Live跳过、Release4项和 Lean PASS。固定探测不再从 ENOENT 推断程序缺失；通过版本化 Binary/Mapping 和原 ToolSpec 指导实际执行位置、程序实现/参数及路径别名检查。旧 generation1 选择按 ADR-0025 保持可读，新请求 generation2 可审计；不新增 GNU/BSD 分类器、容器探测或全量重复检查。受控 CLI 验证真实合成程序输出及 canonical alias，不代表真实模型工具选择质量已通过；仍需 R6 有界 Live。

R4a 已完成独立审计、完整 Gate 和 release 离线验收：通用命令错误保留 program、effective cwd、真实失败阶段和原始 OS 错误；根路径由既有显示 owner 输出 `.`。合成回归验证缺程序/目录/解释器/权限、非零退出及单命令/串行/并行一致性，没有按 ENOENT 猜测原因或改调度器。程序实现身份、平台参数指导和别名说明仍属 R4b；本项未整体关闭，也未追认旧日志为可重放证据。

模型知道主机是 macOS，却调用不兼容的 GNU nm 参数，属于本次具体的规划/工具选择错误。macOS 上也可能安装 GNU/LLVM 工具，不能仅根据 OS 推断同名程序的参数集。

同批次 readelf/timeout 报 ENOENT，nm、docker 或 pwd 却能启动。当前启动错误只突出 cwd 和 OS error，缺少 program/失败阶段；模型误判为目录消失或并发故障。`/tmp` 在本机链接到 `/private/tmp`，这不是两套失踪的目录。成功 cwd 的相对空字符串与失败绝对路径展示也增加混淆。

待讨论：明确主机/容器执行位置及工具实现身份；本 Linux ELF 任务优先在确认可用的 Linux 容器内使用 GNU/PWN 工具。按真实程序帮助纠正参数，区分缺程序、解释器、cwd 和非零退出。没有证据支持改并发调度，也没有足够对照证明整个模型不适合漏洞研究。

### CTF-05：批量命令等待缺乏可见进度（P2）

新 Session `chat-1789142558-987912000-21442-1` 最后执行 serial 的 pwd 和 docker exec。批次收集全部结果后才返回；底层并发 drain 输出，但没有向 CLI 流式呈现，已完成子命令也不可见。

现有 run_command/run_commands 都有 120000ms 工具级超时；后者是整个批次额度。本次约 104 秒后留下 interruption/unknown，未留下 tool_timeout 收据，不能断言超时触发或精确卡在 Docker、bash、挂载、管道哪处。

待讨论：子命令进度、耗时/deadline、已完成输出与批次结果的关系；复用现有 Job/PTY 可轮询能力。进度不能伪造 Receipt；终止 Docker 客户端不等于证明容器内进程已终止。

### CTF-06：Docker 名称冲突与 shell 掩盖失败（P2）

前轮 Compose 启动遇到固定 container_name 冲突；已有 network 创建副作用。后续 compose ps 返回 0，使整段 shell 被报告成功，但不代表容器成功启动。

待讨论：测试环境名称隔离、资源归属、保留关键命令退出码与验证后置状态。不能通过 Docker 错误字符串特判改写通用工具收据。

### CTF-07：测试脚本多行任务未作为一个 Turn 输入（P2）

harness 一次写入多行 stdin，但 CLI 按行提交。故障轮只有首行进入 user History，Goal/Rules 未一并到达这个 Turn；Project Prompt 独立包含部分规则，所以也不能说所有约束都未收到。

待讨论：按已有 CLI 输入契约使用单次完整输入或已有 `/paste` 路径，并用实际请求/History 验证；不因此扩大到非交互执行产品。

### CTF-08：新 Session 复用旧 Project 导致实验污染（P2）

已新建 Session，但 Research、Memory、work 分析文件和容器状态仍复用，模型读取了旧分析材料。不能将结果作为“无先验从零解题”的评测。

待讨论：新 Project 仅放允许的题目输入和环境说明，显式 binary DomainProfile、日志外置、验证挂载范围，继续禁止 exp/wp/现成解法。当前默认 source 属于测试入口配置问题，不应由模型自由文本偷偷切换领域。

### CTF-09：hash 出现多次不等于反复执行校验（P2，效率待测）

最新上述 Session 的 4 个 RequestAudit 对应持久 ToolCall 中，仅有一次 shasum，处理题目、libc、loader 三个文件。Full 请求重发历史，使命令、结果和 hash 在 debug 重复出现；不能按日志字符串出现次数计算执行次数。

内部 hash 另有必要职责：Artifact 内容身份/完整性、目标版本绑定、缓存失效、请求与证据关联。Project Snapshot 的常规刷新传入空 hash_paths，不表示每轮重读全部目标文件计算内容 hash；非 Git 项目仍需元数据遍历。ChatContextCache 每 Step 对部分事实进行精确摘要，变化时以读取前后 revision 验证稳定性；Prompt/Memory 也重读。不可把所有校验当成冗余，亦不能无测量就断言它们是性能主因。

待讨论：分别度量 I/O 字节、hash 耗时、缓存命中、重复请求 token 和模型主动校验次数。目标初次绑定、内容变化与复现交付时校验；复用已验证的不变数据，保持精确来源和外部编辑检测。缓存/增量优化需要证明正确性，不仅依赖弱 mtime 假设。

### CTF-10：测试日志驱动 Snapshot 变化及上下文膨胀（P2）

live 日志存于扫描 Project，FileManifest 包含它们；写日志时不断出现 project_snapshot_diff。日志变化是明确的潜在失效源，但尚未逐文件证明每个变化都由它引起。

待讨论：日志置于 Project 外，区分研究相关变化与测试输出噪声，再评价差分增长与压缩策略。不要用加大窗口代替定位，也不能直接删除有意义的事实变化。

### CTF-11：debug 与 Assistant 输出可读性（P3）

debug 工具定义段已经按单独授权隐藏，实际 ToolSpecs 和 RequestAudit 保留；只减少终端噪声，不节省模型 token。已有 13 项离线测试结果，仍为未提交本地变更。Assistant 多步输出直接拼接、Full 历史反复显示的辨识问题仍待讨论；显示简化不能伪装成精确完整 wire 抓包。

### CTF-12：外部材料获取时机与能力边界（P2，方案议题）

有具体知识缺口时按需获取工具文档、准确版本源码、CVE/修复差异，可与独立本地检查并行。ENOENT/平台差异先确认环境，不应泛搜漏洞。当前 tool_search 是内部工具发现，http_request 是已知 URL 获取，不等于互联网搜索引擎。

待讨论：外部搜索入口是否必要、具体查询触发条件与来源/版本记录。继续禁止本题 WP/EXP/现成解法；外部材料不直接成为本目标漏洞证据。尚未决定接入搜索服务或新增依赖。

## 追加只读代码检查（2026-09-12）

以下来自当前实现检查，未开展新的 Mock/Live 重现。固定限制与数据流可由代码确认，实际长任务触发频率及性能影响仍需后续验证。

### CTF-13：单 Turn 固定 32 次模型请求后直接失败（P2）

`src/app/agent_loop.rs` 的 MAX_MODEL_REQUESTS_PER_TURN 为 32，达到边界返回 ModelRequestLimit。CLI 对 AgentLoop Err 直接结束 Chat。这是独立于 1M token 窗口的步数限制，不应把扩大上下文理解为可无限持续工作。若最后一步请求仍产生工具调用，后续可能已执行并保存结果，却没有剩余采样用于总结。

待讨论：保留运行预算，但明确剩余 Step、结束边界与可继续的停止状态；配置与预算策略应有唯一所有者。不要简单删除上限，也不要引入自动无限续跑。此问题未在本次 4/13 Step 的失败轮中触发。

### CTF-14：命令输出截断丢失原始尾部，不只是 UI 预览（P2）

`src/adapters/tools/command.rs` 的 read_bounded_output 继续 drain 进程管道，但只保存限额内前缀，超过限额的字节不进入返回值；output_value 返回文本和 truncated，没有该命令完整输出的 Artifact 引用。对这个捕获路径，之后审计只能保存已捕获的内容；不能从 truncated Result 还原原始尾部。

默认输出总预算为 128 KiB；批次按子命令数均分，再为 stdout/stderr 各分一半。32 条命令时每条 stdout 仅约 2 KiB，尾部诊断、崩溃信息或结果容易丢失。显式让命令重定向文件可能避免这一问题，但不是当前捕获路径自动保证。

待讨论：将模型预览额度与原始输出保留策略区分，复用现有 Artifact 或明确输出文件；评估头尾保留、截断字节数、来源引用及磁盘限额。不能仅增加所有请求的输出上限，也不能声称现有展示截断都保留了完整原文。

### CTF-15：批次中已完成子命令缺少中断前的持久检查点（P1/P2，需回归）

run_serial 把每条结果累积在局部 Vec，整个 run_commands 返回后才产生批次 Outcome/Receipt。工具级超时丢弃执行 future 并返回取消 Outcome；主动中断时 scheduler 保留未知状态，不制造 Receipt。因此若前几条已成功、后续一条阻塞，中断会丢掉前几条尚未返回的内存结果；真实副作用可能已发生。

这比 CTF-05 的“屏幕没有进度”更深：仅加 UI 输出不能解决可恢复性。现有不伪造执行事实原则正确，缺口是批次内部完成粒度。

待讨论：短小批次的适用范围、独立 ToolCall/Job 的使用边界，以及是否需要在既有事实体系内表达部分完成。需专门覆盖“第一条产生可观察结果、第二条阻塞后中断/超时”，不得把所有子命令都记为未执行或自动重跑整批。

### CTF-16：上下文压力提示与实际交付成本不是同一口径（P2）

`src/cli_chat/compaction.rs` 的 ContextPressure 从 ContextDecisionManifest 候选估算需求构建；它是逻辑 Context 视图，不是实际 Provider delivery 的完整计量。AgentLoop 在后续步骤还可能保留 reasoning 与累计状态 diff。提示也在 Turn 结束后才显示，`/context`、`/compact` 由同一串行输入循环处理，不能在长 Turn 中即时使用。

实际 delivery 仍有预算校验，这不是已证明的越界发送；问题是用户可见压力和运行中干预时机不足。

待讨论：区分 logical demand、actual delivery estimate 与 Provider 实报 usage，按 Step 观察趋势并在有用的时机提示。自动 Compaction 目前不在冻结契约中，不能未经方案决策直接加入；若讨论安全停止/人工压缩，须保持工具配对和续接边界。

### CTF-17：后台 Job 的进程内 ID 在重启后可复用（P2，潜在恢复歧义）

`src/adapters/tools/job.rs` 的表是进程内状态，next_job_sequence 从 0 开始，生成 job:1、job:2 等；`job/control.rs` 根据 job_id 查当前表。历史对话可保留旧 ID，新 Runtime 又可能分配相同 ID。

代码可确认编号重用和进程内查找，但本轮没有复现“旧操作命中新 Job”。历史 ID 若被继续使用，存在误指向新任务的潜在歧义，需检查 Resume/新任务组合。

待讨论：明确运行实例身份、历史句柄失效和重启后不可恢复的边界。优先做小型身份绑定，不建设守护进程、分布式队列或假装能恢复已丢失的进程控制权。

### CTF-18：错误分类未转换为足够好的交互恢复体验（P2）

CLI 对 run_turn 返回的所有 Err 直接退出 Chat，无法在同一界面检查状态、调整输入或选择如何继续。模型请求失败、预算/步骤边界和持久化错误有不同风险，不能统一自动重试，也不能统一吞错继续使用不一致状态。

待讨论：区分可恢复停止、需要重建 ProviderSession 的失败、必须退出的持久状态错误，给出真实检查点和下一步操作。保留 Ctrl+C 后未知副作用语义；是否允许继续须基于状态一致性，不靠泛化 try/catch 或隐式重放。

## 方案讨论完成前不作出的结论

- 不把 Prompt 续接失败定为已证实的 1M 容量耗尽。
- 不把程序缺失定为已证实的 cwd/并发故障。
- 不把模型文字、shell exit 0 或进度显示当成最终效果证明。
- 不把重复 debug 历史当成重复工具执行或重复文件 hash。
- 不凭一次失败评价模型整体能力，也不认为只换模型即可解决工程问题。

后续先对齐方案、职责归属、验收回归与成本，再确定统一修复范围。本文不提前创建新 Slice、抽象、配置或永久 API。
