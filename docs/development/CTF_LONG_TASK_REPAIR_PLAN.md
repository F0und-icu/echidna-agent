# CTF 长任务可靠性修复计划

## 2026-09-12 连续执行授权增补

操作者已授权完成本次全部修复、修复过程独立审计、Live Test 与远端提交。此授权替代下文历史“仅首包/另待授权”的日常开工停止条件；不替代逐包 Scope Lock、最小 ADR、独立审计、完整 Gate/Lean 或既定 Live 限额。主 Agent 顺序实现，独立 reviewer 不得为实现者。Actions 继续暂停，提交用 `[skip ci]`，不推定 PR merge/force-push/Tag/Release 授权。各包完成后推进下一包，不因本地封存自行结束整个修复系列。

L1/L2 仍用第 11 节的固定上限与隔离要求。凭据只从显式 Cyber Agent 配置读取；禁止本题 WP/EXP/答案、Codex 登录读取、未知容器接管或删除。必要外部输入缺失时先完成不依赖它的工作再报告。轻量检索只按 R5 测量准入；重型知识库/向量服务仍 Deferred，不把它夹带进可靠性修复。

后续明确答复已授权核验后普通合并 #68/#69 和通过验收的修复 PR；不绕过必需检查，不 force-push，不运行 Actions。准确提交与状态见 STATUS 顶部，下文首包开工边界为历史记录。

- 日期：2026-09-12。
- 状态：首个 Slice `CTF-R1` 已获本地实现授权；仅覆盖 R0 合成验证前置与 R1 Resume 配对。其余工作包仍为候选；独立审计待单独授权，不含 Live 或 Git 操作。
- 问题依据：[CTF_LONG_TASK_ISSUES.md](CTF_LONG_TASK_ISSUES.md)，CTF-01 至 CTF-18。
- 当前本地基线：`codex/f6a-vision-coordinate-contract`，HEAD `b10af4e897254c066aac7b560b2656fb2965d39c`。远端状态仅沿用 STATUS 历史记录，本轮未刷新 GitHub。
- 保留现有 debug 工具定义隐藏改动及对应测试；它未完成独立审计/完整 Gate，不算本轮完成项。
- 原测试目录及旁路 issues 文件本轮均已确认不存在。旧结论保留为历史证据，不宣称可直接重放原日志；先补无敏感内容的最小回归。
- Codex 桌面聊天序号修复是独立的本机维护事项，用户已报告恢复；不计入 Cyber Agent 产品修复，也不继续访问 Codex 登录/聊天数据。

## 1. 目标与边界

本轮目标不是增加工具数量，而是使现有交互 CLI 能稳定完成较长的漏洞研究：输入完整、环境明确、执行可观察、结果可追回、停止可解释、恢复不重复副作用。

保持单 package、单 AgentLoop、Config v1 唯一装配、协议通用、FullAccess、无遥测和现有 Evidence/Receipt 权威。保留文本/单多模态/双 Provider 模式。不新增 TUI、非交互执行、Benchmark Adapter、Sandbox、搜索服务、自动压缩、自动重试、角色目录或通用 Hook。

下列 R0–R6 是工作包，不自动成为已授权 Slice。当前只启动 STATUS 锁定的 `CTF-R1`；每包开工前登记允许文件、基线、责任人、测试、ADR 和停止条件；有跨边界大 diff 时按列出的子包拆开验收。

## 2. 顺序与覆盖

| 工作包 | 修复目标 | 问题覆盖 | 前置条件 |
| --- | --- | --- | --- |
| R0 验证基线 | 干净测试项目、完整输入、环境和日志隔离、最小故障样本 | 06、07、08、10 | 文档/测试工作授权；不调用模型 |
| R1 Resume 配对 | 修复开放 Call 越过后续消息的交付顺序 | 01 | R0 最小样本；无环境操作依赖 |
| R2a 续接诊断 | 区分上下文预算、Prompt/Memory 替换与 Provider 续接失败 | 02、16 | R1；先用可控样本确定分支 |
| R2b 有界继续 | CLI 可恢复停止、请求额度、明确人工继续 | 13、18；消费 02/16 | R2a；恢复契约先定稿 |
| R3a 输出与句柄 | 原始输出保留/截断边界、旧 Job 不误命中新 Job | 14、17 | R0；先于新的 Job 引导 |
| R3b 执行检查点 | 子命令部分完成的耐久性及真实进度 | 05、15 | R2b、R3a；增量契约审查 |
| R4 环境与工具指导 | 程序身份、执行位置、准确错误、按需资料获取 | 03、04、06、12 | R0；长任务路径依赖 R3 |
| R5 展示与效率 | Assistant 分段、debug 标识、测量后优化重复成本 | 09、10、11、16 | R2–R4 数据口径稳定 |
| R6 分级真实验收 | 先合成短 Live，再一个隔离 CTF 长任务 | 对全部问题复核 | 对应离线 Gate/审计通过，单独 Live 授权 |

建议串行开发顺序：R0 → R1 → R2a → R2b → R3a → R3b → R4 → R5 → R6。纯环境说明可在 R0 纠正，不必等待 R4 的产品改动。只在明确授权委派后才并行不重叠的工作。

## 3. R0：先让测试结论可信

工作分为永久回归 Fixture 与目标专用外部 harness，不能混为一套 Benchmark 产品。

- 永久合成回归放顶层 `tests/`，不得复制旧密钥、目标二进制、实际请求或 WP/EXP。
- 每个从零解题试验用新 Project + 新 Session，仅复制 allowlist 中的题目文件、必要依赖及环境说明；禁止读取/复制 exp、wp、writeup、现成解法和预存 flag。不按“删掉几个文件名”推定目录干净。
- harness 使用已有 CLI 多行输入契约；以持久 User History 和实际首次 RequestAudit 验证整个目标/规则确实进入同一 Turn，不能只检查 stdin 写入成功。
- 显式 `binary` DomainProfile；日志、harness 和配置在被测 Project 外。写测试日志不能制造被测 Project Snapshot 差分。新 Session 不等于干净 Project。
- Docker 资源按运行隔离命名和标注归属；不接管/删除未知旧容器。逐项保留启动结果及后置状态，不能用最后一个 `ps` 的 exit 0 掩盖前项失败。
- 环境说明区分主机 macOS 与容器 Linux，删除对主机缺失命令的强制要求；工具缺失先记录，安装/拉镜像/容器变更另行授权。
- 原始日志保留到问题闭环和验收之后，轮次独立、禁止覆盖；报告引用运行 ID、提交、配置语义指纹和 Session ID，不保存凭据。

验收：无模型 Fixture 证明多行输入、目录白名单和日志隔离；形成 R1–R3 的最小故障用例清单。目标材料检查只能在获得对应路径使用授权后进行。

## 4. R1：History 归一化只修一个所有者

入口：`src/domain/history.rs`；受影响边界为 Session Resume、Compaction、Context 和 Responses 交付。不得在 CLI/Adapter 再实现一套配对。

- 将确实无真实 Result 的 Call 的派生 unknown 占位放在对应开放调用组之后、后续用户消息等不兼容交付边界之前，而不是整个 History 尾部。
- 先确认完整历史中的真实配对，避免扫描到中间位置就误补占位；保留并行 Call 组及真实 Result 的合法相对顺序。
- 有真实结果但时序本身不合法的历史，必须诊断或按明确的交付规则处理，不能悄悄移动真实执行事实来伪造因果。
- 占位仍只是派生恢复标记，明确 `execution_status=unknown`；不生成真实 Receipt，不写回旧 Event，不证明无副作用，不自动执行。已存在 Receipt 但缺失其 Result 的耐久异常仍按既有 Session 边界拒绝。

必须回归：`user → open call → later user`、多个 Call 部分完成、尾部 open Call、重复 ID/name mismatch、迟到真实 Result、重复 normalization 幂等、Resume→Full wire 精确顺序、Compaction 保留完整对、原 Session bytes 不变。通过 Mock 断言发送序列，不能仅比较归一化后长度。

优先复用：`tests/history_pairing.rs`、`session_resume.rs`、`session_compaction.rs`、`responses_http_adapter.rs` 与 `cli_chat_loopback.rs`。本包不改变 reasoning 支持或 Provider 类型。

## 5. R2：把“失败退出”变成准确、有界的恢复

### R2a 先定位和计量

入口：`src/app/agent_loop/context_state.rs`、`agent_loop.rs`、既有 ContextPlanner/CLI context 与 compaction owner。

- 给现有拒绝路径携带稳定的 typed 原因：逻辑预算不足、实际交付候选超预算、Prompt/Core Memory 替换、History/其他基线不兼容等。名字在实现 Scope Lock 固定，不靠错误字符串或模型名称分支。
- 三种口径分开：logical demand estimate、实际 delivery estimate（包含必须保留的 reasoning/差分）、Provider 实报 usage。不可用为 unavailable；bytes 不冒充 tokens。
- 用四个独立 Fixture 隔离 Prompt 热编辑、Core Memory 热编辑、只增加合法 Project/Research diff、只增加 reasoning/交付长度，再做组合用例。历史失败只有组合线索，未重现前不能宣称唯一根因。
- 复用当前预算校验值和 RequestAudit/CLI 观察路径，不造第二个估算器、事实日志或监控服务。按 Step 在有意义的变化/阈值时提示，不每个 token 输出。

### R2b 再实现恢复与请求预算

- 固定分类为：可回到输入提示的有界停止；需要重建 ProviderSession 的失败；必须停止写入/退出的持久性或完整性错误。分类由 App 的状态与错误决定，CLI 只呈现并选择已有恢复路径。
- 预算到达、兼容性拒绝和纯请求失败，只有在已完成结果持久化、无未处理耐久错误的情况下才能继续交互。返回提示不等于复用一个已经失效的 ProviderSession。
- 未知在途工具、Receipt/Result 不一致、持久化失败，不能被泛化 catch 后直接继续；提供检查点和检查/恢复路径，不自动重试旧调用。
- 请求次数使用一个运行预算所有者，通过现有配置装配可设置；保留兼容默认值 32，明确总实际请求和 Primary/Vision 的记账关系。配置值校验及小额度边界先测，不添加无限开关或隐藏重置计数。
- 最后一次采样产生的工具调用按已接受执行规则完成/停止并保留结果；额度耗尽后不偷偷追加“总结请求”。显示已用/剩余额度和明确“继续”的新 Turn 边界。
- 默认仍手动 `/compact`；不能为了继续而丢 reasoning、自动压缩、忽略新 Memory 或重复工具。保持 F3 每 Step 刷新规则：不兼容变化先解释并停止，用户显式恢复后应用当前来源。
- 将 Prompt 加载改为整 Turn 冻结或队列生效，是另一项契约变更候选，不在本计划中默许实施。先用诊断结果证明其必要性，再写 ADR；不能混同固定的 Domain/Phase Turn 边界。

验收：请求额度 1/2/32 及超过旧 32 边界的显式配置；预算停止后 CLI 仍可用；显式继续不复制 user input/Tool execution；失败响应不推进 baseline；持久化失败仍 fail closed；Full-only、增量、Primary/Vision 隔离和显式 Compaction 均有回归。

## 6. R3：执行结果不能只活在内存里

### R3a 原始输出与 Job 身份

R3a1 已由 #73 合并。R3a2 按已接受 ADR-0023/v1.13 完成本地审计/Gate/release/Lean：单流1 MiB、单前台批次8 MiB 静态分配，原始字节与预览分离，App 固定 Receipt 前发布，v2 收据引用/兼容和 read_output 回读。准确交付状态见 STATUS。限制是每执行保留上限，不是项目累计磁盘配额；运行中和未观察到的执行仍不承诺耐久，R3b 必须单独验收。

执行拆分：R3a1 先按 ADR-0022/v1.12 完成 Job/PTY 运行身份隔离，独立验收与交付；R3a2 再落实输出 Blob/完整性/限额及执行后持久化失败边界，契约定稿后编码。这样避免把纯句柄修复、输出 Schema 和跨 Port 失败模型放在一个累计审计 diff 中。R3a1 不关闭输出丢尾或批次部分结果问题。

入口：`src/adapters/tools/command.rs`、`output`、`job`，以及现有 Artifact/ToolResult/CLI 消费路径。

- 将模型/CLI 预览额度与原始捕获保留额度分开。模型得到有界头尾片段、截断说明和可回读引用，大输出不全部塞进 Context。
- 原始捕获复用已有 Artifact/Blob 能力，并设磁盘/字节硬额度；额度内保证可回读，超额度明确 partial/truncated。继续 drain 防止管道阻塞，不承诺无限完整存储。
- stdout/stderr 分别记录实际观察/保留字节、完整性及来源；进程仍运行或采集被中断时，总字节数可能未知，不伪造最终总量。非 UTF-8 原始字节不因展示转换丢失。
- Artifact 提交失败是持久化错误，不能重跑已完成命令。后台 Job/PTY 只共享真正相同的捕获语义，不借机统一整个进程运行时。
- Job ID 绑定运行实例身份，历史句柄永不匹配新实例的 Job；旧 ID 在历史里可展示，但操作明确过期。新运行 ID 使用已有适合的身份生成能力，不引入 daemon/队列，不假装恢复旧进程控制权。顺带静态核查 PTY 句柄是否同类问题，有事实才扩展 Scope Lock。

回归：超预览限额的尾部标记仍可从 Artifact 读到；32 子命令不再静默丢尾；非 UTF-8、磁盘限额/失败、超时/取消完整性；两个运行实例的旧 ID 不能 poll/input/stop 新 Job。

### R3b 部分完成检查点，然后才做进度 UI

按有限审计边界拆分：R3b1（ADR-0024/v1.14）固定 Port/App/Session 子项检查点与恢复已经 #75 合并；R3b2 消费真实检查点的进度展示和慢终端回归已通过独立审计、完整 Gate、release5项及 Lean。R3b 离线验收完成，R3b2 远端交付待核验；不以此关闭 R4/R5 或 R6 Live。准确状态见 STATUS。

- 先重现“第一条命令完成且有可观察结果，第二条阻塞，随后取消/超时”。分别检查输出、真实副作用和 Session 可恢复内容，不能仅断言总批次 Cancelled。
- 推荐在现有 Session/Event + Artifact 体系表达子命令已观察结果，绑定父 execution_id、子项序号及输出来源；完成子项的检查点必须在可能丢弃外层 future 前持久化。
- 新的子项检查点不是整批成功 Receipt；最终批次仍只有原 ToolCall 对应的结果/收据规则。未观察子项保持 unknown，确认未启动者才可标 not_started；不自动重跑整批。
- 部分结果的 Port 传递、Event 版本、父/子终态、取消与写入竞争需要先写有生产消费者的最小 ADR；不能直接从 Adapter 向 Session 写事件，也不能为每块 stdout 创建持久事件。
- 展示区分 started、streaming/uncommitted、subcommand checkpointed 与 batch terminal；有界显示子项进度、当前命令、耗时和 deadline。慢终端/输出背压不能阻塞管道 drain 或改变执行结论。
- 长运行/交互任务优先用现有 Job/PTY；普通短命令仍用 run_command(s)，不得自动把模型已请求的执行转换为后台任务。修改指导仅减少风险，不算替代 CTF-15 的耐久修复。
- Docker 客户端终止仅说明本地观察停止，不证明容器内进程已死。

回归：serial/parallel 部分完成、工具级超时、Ctrl+C、进程终止后的 Resume、检查点持久化失败、慢 UI、输出上限；已观察完成结果保留，未知结果不伪造，执行次数不因恢复增加。

## 7. R4：让模型基于实际环境选工具

R4b 采用最小修复：复用现有 `probe_failed` 而不把 ENOENT 推断成 `missing`；不新增输出字段或固定工具探测项。程序路径/实现/参数及容器身份按需通过普通命令获取，不以 host OS、单独版本数字或 fingerprint 推断。Binary/Mapping 资源升级的历史选择兼容性由 ADR-0025/v1.15 约束；新请求及 ToolSpec 进入既有 hash/audit，机械 Replay 不重写旧字节。路径别名是可观察事实，不硬编码映射规则。离线夹具验证交付，不证明模型会正确选择每条命令。

实现拆分：R4a 先修 command 唯一执行 owner 的失败诊断（program/effective cwd/process stage/原始 OS error），并由既有 path-display owner 修正空根路径为 `.`；保留现有结果 Schema/error string、退出码、收据和重试语义。R4b 再处理环境观察和按需工具指导，变化的 Schema 必须先单独说明兼容性。R3b2 已经 #76 合并；R4a Scope Lock 见 STATUS。永久测试以行为命名，不新增 CTF 模式或预定解题工作流。

- 复用现有 `inspect_environment` 和 command execution owner；主机 OS、架构、实际 cwd、程序路径/实现/版本和容器观察要分开。
- 不新增通用探测 DSL。初期任务特定可用性检查通过现有 run_command(s)；只有重复使用证明收益，才给固定 environment owner 增加有界探测。一次环境观察不能代表容器内全部工具。
- command 启动错误包含请求 program、有效 cwd、失败阶段和原始 OS 错误；ENOENT 可能是 cwd、可执行文件或解释器，不在证据不足时硬分类。退出非零与 spawn 失败分开。
- 展示 Project 相对路径及必要的 canonical root，说明 `/tmp` 与 `/private/tmp` 别名，不修改 OS 路径语义。
- binary/mapping 和对应 ToolSpec 加少量规则：确认实际执行位置/必要工具再展开；Linux ELF 优先使用已验证的 Linux 工具环境；参数以实际工具实现为准；短独立检查可批量，长任务用 Job/PTY。不堆叠 Prompt，不自动改写用户输入/argv，不增加人格。
- 外部获取由明确缺口驱动：API/命令参数查版本匹配文档，源码行为查精确版本，漏洞背景查原始公告。记录来源与目标适用性；已知 URL 用 http_request，tool_search 仍只是内部工具发现。搜索引擎接入 Deferred，禁止查询本题 WP/EXP/现成解法。

验收：受控 macOS/GNU/LLVM 工具身份差异；缺 program、坏 cwd、缺解释器、非零退出；一次失败不被报告成并发损坏；环境不变不强制反复全量探测；Prompt/ToolSpec 更新进 fingerprint/audit 回归。

## 8. R5：展示改进和测量驱动的效率优化

当前拆分：R5a/R5b1/R5b2 已合并。R5b1 的 Tool 边界测量见 [R5B1_SNAPSHOT_MEASUREMENT.md](R5B1_SNAPSHOT_MEASUREMENT.md)，R5b2 正式 Chat 测量见 [R5B2_CHAT_DELIVERY_MEASUREMENT.md](R5B2_CHAT_DELIVERY_MEASUREMENT.md)。后续核验确认显式 `hash_paths` 在同大小/mtime 修改后可能返回旧值，命中及重建均受影响；当前 R5b3 按 ADR-0027/v1.17 在原 owner 修复，验收状态以 STATUS 为准。内部 hash/扫描次数、Provider usage/TTFT、优化收益及 R6 仍未完成。历史测量保留为修复前基线，不据此封存整个 R5。

- 收口现有 debug 工具定义隐藏改动：独立审查后走完整 Gate/release 检查，不重写已有实现。
- Assistant 每个 Step/消息边界可辨，流式正文不黏连；Full 与 incremental 请求标识清楚。默认不重印大段旧历史，但 debug 若采用折叠/省略必须显式说明，不能声称是逐字 wire 抓包。
- 区分 UI 预览截断与原始捕获截断，显示 R3 的可回读引用；release 排除 debug、保留实际 Provider reasoning。
- 对 hash/Snapshot/Context 的优化先固定测量样本：hash 次数与字节/耗时、目录扫描、cache hit、RequestAudit 大小、delivery estimate、Provider usage、TTFT/总耗时与重复工具执行次数。无持久信号的指标为 unavailable，不按 debug 字符串计数。
- 先排除日志污染，再测稳定 Project、仅目标改变、仅 Memory 改变、日志外置增长四组。只复用能够证明未变的数据，不放松精确内容身份或外部编辑检测，不只靠 mtime。
- 默认在外部本地测试报告计量，不建设新的产品监控或第二份 Trace。只有确实影响运行决策的信号才进入既有观察路径。
- 优化合并条件：固定样本结果正确、重复扫描/hash 或交付量实际减少，且计入缓存自身成本；无收益实现删除或 Deferred，不做“为了缓存再加缓存”。

R5b3 已由 PR#82 合并。当前 R5b4 仅评估并优化既有 Blob 字节发布的重复临时写入：先精确校验已存在 Blob，再决定是否需要 staging；不减少内容完整性校验、不新增缓存。使用原正式 Chat 四场景做修改前后测量，真实字节发布消费者做命中无目录变更与损坏负例；验收与交付见 STATUS。Tool wire 省略、Context 复用、检索提升与 R6 均不混入本包。

## 9. 架构与治理准入

### 轻量检索 / 知识库边界（计划增补，尚不实施）

- 先完成 R0–R4 的可靠性路径，R5 再评估项目内 Memory/Research/Artifact 元数据的轻量检索。复用既有 exact identifiers、BM25、CJK/ASCII n-gram、RRF 与资格/权威层，不新增第二套检索所有者。
- 先在固定离线样本做 Shadow，测相关引用命中、误召回、重复读取、Token/I/O 与索引自身成本。只有可重复的净收益才另行批准进入现有 ContextPlanner；检索不绕过预算，不成为事实来源，不自动执行建议。
- 个人只读参考资料目录属于后续候选新来源：启用前单独 ADR/配置授权，记录来源、版本、适用条件和精确引用位置。参考知识与项目证据隔离，不自动跨项目检索私有研究；相似 CVE/文章只能提供线索，不能证明当前目标有漏洞。
- 源码定位继续优先精确路径、符号和调用关系；不以全源码切块 RAG 替代验证。向量库、embedding、远程 reranker/judge 暂缓，只有词法检索缺口和实测收益足以抵偿维护成本时再评估。
- CTF 检索语料按 allowlist 固定并记录版本，排除本题 WP、EXP、答案和预存 flag；不得借知识库引入解题材料污染。本 Slice 只补计划，不创建知识目录、索引、配置或产品 API。

### 各包准入

本计划不修改冻结 v1–v1.10，也不追认现有实现通过。按现有职责修正实现缺陷无需发明新架构；改变契约时必须先完成 ADR 与增量架构决策。

必须在相关包开工前明确的设计门：

1. R2b 的可恢复停止状态、ProviderSession 重建及新运行预算配置的所有者/兼容性。
2. R3a 的输出 Artifact 完整性/限额 Schema，以及 Job 运行实例身份和 legacy 操作边界。
3. R3b 的子项检查点 Event/Port、取消原子性与 Replay 规则。
4. 如实际改变 R4 环境工具 Schema，或改变 Prompt 更新生效时机，独立说明版本/旧请求回读，不静默修改冻结语义。

新增契约必须有当前生产调用方；禁止只为测试新增 pub、第二个 normalizer、第二个预算/错误映射 owner、Hook/路由注册表。默认零新 crate；若现有 Artifact/进程路径不足，先比较最小补齐成本，不能直接搬 Codex 大模块。

每个实际 Slice 固定顺序：专项测试 → 受影响测试 → 独立审计 → 修复 → 完整 Gate → Lean Review。修复后受影响回归重新验证，重大变化需要有限复审；审计超时/无结果不记 APPROVE。

委派只有另获当前授权才启动：Terra xhigh 负责 R1/R2/R3 跨边界；Luna xhigh 只负责明确的 Fixture、文档或单模块展示/Schema 小改。独立 reviewer 不能是实现者，有限 diff/明确清单、十分钟审计上限；超时缩小范围或交还主 Agent，不把大累计 diff 丢给 Luna。

完整 Gate：固定 Rust 上 `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cargo test --locked --all-targets --all-features`、`cargo build --locked`，加治理 inline-test 扫描。涉及展示时另做 release 行为检查。永久回归留顶层 tests；不运行 Live 来代替离线 Gate。

## 10. Git 与交付

- 不把本轮计划授权扩展为 commit/push/merge。当前 #68/#69 的 stacked 历史先只读核对，正式交付时再决定经授权合并基础 PR 后从 main 开短分支，或明确记录 stacked base；不直接叠一条无法审查的大修复 PR。
- 现有未提交源码与本轮计划文档分别列 allowlist，禁止 reset/clean、全量暂存、静默 stash 或覆盖用户变更。
- 一个已批准 Slice 一个短 `codex/` 分支/可解释提交；没有 Gate/独立审计不得封存。分批交付，不等所有问题修完才积攒巨大 diff。
- Actions 继续暂停。只有另获 Git 发布授权时才沿用 `[skip ci]`，记录本地 Gate，不将跳过 CI 写成远端测试通过。合并若仍有必需 CI 检查，先获得对应豁免/运行授权，不擅自改分支保护。
- 无本轮 Tag/Release/架构远端 hash seal。目标 Session、恢复备份、日志、真实配置与密钥永不提交。

## 11. R6：分级 Live 与停止条件

每级开始前单独确认 Provider 配置、请求数/时间预算、项目/输入范围及日志位置；全部走正式 Chat 和唯一配置装配，不访问 Codex 登录，不加厂商分支。

**L1 合成协议/恢复 Live**：R0–R2 离线封存后即可申请，不必等所有性能优化。建议最多 3 个合成 Session，每个最多 6 次实际模型请求、5 分钟，无自动重试。覆盖 Full-only Tool loop、中断后显式 Resume、兼容性/额度停止后的人工继续；未知工具结果路径先用确定性离线 Fixture 证明。不得把一次通过当成全窗口容量证明。

**L2 一题长流程**：R3/R4 和阻断验收的展示修复封存后申请。先一个明确授权的 PWN 题目，新 Project/Session，无先验材料；建议总计 30 分钟、48 次实际模型请求上限，跨 Turn 人工继续也计入总量。使用容器需独立确认资源范围。多模态仅在题目实际需要且另定额度时启用，不为了验收强造图片步骤。

分别报告工程结果和研究结果：

- 工程：完整任务进入模型；Resume 请求合法；不重复已执行副作用；已完成子命令与输出可回读；停止原因准确；CLI 能按允许边界继续；日志不引入 Snapshot 噪声；旧 Job 不控制新进程。
- 研究：目标/环境绑定、假设与排除路径、真实验证和阴性/环境对照、可复现程度、消耗及是否完成题目。解题成功不自动证明工程正确，流程稳定也不证明解题能力占优。
- 费用/容量：记录实际 usage 或 unavailable；未实测 1M 不标为通过。长任务达到预算即停，不以“快成功”为由自动扩额。

任何重复执行、耐久结果丢失、非法交付序列、身份串扰或无效证据提升立即停止该 lane，保留新日志、生成最小回归、修复并重新独立审查；不得边测边热改 Prompt/代码后把多个版本算作同一次通过。

CTF-09/12 等尚无量化需求的优化可明确 Deferred，不强塞进稳定性里程碑。CTF-01/02/13/14/15/17/18 的计划验收不能仅以“模型这次没触发”关闭；已重现问题必须有对应永久回归。

## 12. 当前授权入口

操作者已授权更新计划并开始修复，首个正式 Slice 为 **CTF-R1：R0 合成前置 + R1 配对**。本包只构建无敏感内容的离线回归，不宣称已修复真实 CTF harness/Docker 隔离；后者仍在 R0/R4 待办。实现由主 Agent 直接承担，专项/受影响测试之后等待独立审计授权，不继承历史 Live/子 Agent/Git 授权；结束后不自动启动 R2。现有 debug 改动保留、独立列账。
