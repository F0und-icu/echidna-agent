# R6 普通工程长任务验收：首轮阻断记录

## 结论

**STOPPED_BLOCKER_FOUND，R6 未通过。** 2026-09-18 在已合并 main `5c073086bb41cae048dcb67acbe0e4a47d711816` 上开始普通工程 Live；第12次真实请求完成后，下一候选请求在本地因 `BASELINE_REJECTED: ToolsChanged` 被拒绝。停止后不追加模型请求、不热改产品或重新解释验收为成功。

这不是 DeepSeek HTTP400、Tool Call/Result 丢失或1M窗口耗尽。本次已发送12次请求全部 HTTP200/SSE completed；未发送第13次请求。它暴露的是 Full-only 推理续接、项目状态差量与 Deferred Tool 激活的组合兼容缺口。精确不兼容时拒绝而非丢弃推理是正确防线，但普通长流程因此无法完成。

## 固定范围与证据

- 操作者授权：普通工程、最多1800秒/累计48次实际请求，恢复后的请求也计入；新额度不继承旧 L1。未使用的额度不是自动重跑许可。
- 一个全新 Project/Session：`chat-1789743503-748331000-41307-1`；所有工程、正常退出、显式恢复阶段共享此 Session。
- 使用正式开发 CLI `chat --debug`、Config v1/唯一 Provider 装配；Primary `deepseek-v4-flash`、Responses、显式 `incremental_input=false`、Vision disabled、声明窗口1048576、每Turn准入8次。后者是受控压力设置，不改变产品默认32。
- 重新 `cargo build --locked` 成功。Live二进制 SHA256 `c342c91bcd035e66280a4b61fe69ccfbfb1f58ce6379c7be16c73508f46112ae`，真实模型测试期间至脚本finally核验时未变。停止Live后的只读诊断运行现有Cargo集成测试，Cargo重新生成开发二进制，当前文件SHA256为`50cc18908df2752068374c49886d18a4c715583aae7a921538c054c66d8da69f`；未用此后生成的二进制发送任何Live请求。未新增生产代码、依赖、Port、API、Schema、产品服务或长期测试模块。
- 外部证据：`/tmp/cyber-agent-r6-live.RrchBH/`。包括 `plan.json`、各阶段固定输入、stdout/stderr、`request-01.json`至`request-12.json`、`wire-observations.json`、`summary.json`、`facts-verification.json`、`oracle-summary.json` 和正式CLI最终检查输出；真实日志/配置/Session不进Git。
- 验收专用 localhost 转发计数器保留实际请求体，不改语义；只转发至显式原URL，验证上游TLS，不重试/跳转。成功HTTP返回证明本次12次上游请求均到达服务端；计量不以 RequestAudit/准入数冒充网络次数。此转发层是测试条件，不是产品协议实现或无转发性能基线。
- harness SHA256 `de13da09cd0b0f6e84588e2d1571dcc5592ae53d9a24d2ba8485e721ea2880e1`。预检曾因外部脚本未创建 Memory 父目录失败；零网络请求，证据保存在 `preflight-zero-requests/`，只修正脚本后才冻结并开始 Live。
- 原 Cyber Agent 凭据文件未变；临时0600配置已删除，保留证据中未检出该精确凭据。未访问 Codex 登录/配置、未启动 Actions、Docker、安全题目或远端交付。

## 场景与实际覆盖

小型 Python 标准库账单汇总程序包含已知的币种、Decimal舍入、数量、去重及校验缺陷。允许修改 `ledger.py` / `REPORT.md`，不准修改固定测试或夹具，不准联网/读取父目录。外部 oracle 不向模型公开。

| 检查 | 实际结果 |
|---|---|
| 环境检查与普通工具循环 | 先调用 inspect_environment/list_files；观测Python3.12.4，使用python3验证；未见GNU参数误用 |
| 任务修复正确性 | 原7项测试产生16个失败断言；修复后7项通过，另5项外部校验通过；固定测试/脚本哈希未变 |
| 第一次预算停止 | 8次准入用尽后停止；已接受工具操作完成持久化，无自动追加总结请求 |
| 显式恢复 | 新CLI进程 `--resume` 并输入 `/resume` 后用2次请求完成；并非只验证同进程热恢复 |
| 文件/Memory新鲜度 | 退出后等长修改settings USD→EUR、Memory locale en-US→fr-FR并恢复mtime；再次Resume读取最新值 |
| 当前样例结果 | USD为`{"alice":"20.01"}`，新EUR为`{"alice":"0.22","bob":"7.04"}`，与独立oracle一致 |
| 协议与持久事实 | 12次Full请求均通过配对顺序检查；22 Call/22 Result/22 Receipt、6个真实子命令检查点；无未配对调用 |
| 原始数据完整性 | 校验696个请求组件引用、16个捕获引用，涉及105个不同Blob，大小/SHA256均匹配；正式Session show/export与最新prompt inspect通过 |
| Deferred激活后续采样 | **阻断**：成功搜索read_output后因ToolsChanged与差量续接组合拒绝下一请求 |
| 一次性副作用/大输出中段回读 | 未到达；commit_once.py与emit_output.py均未执行，不能宣称通过或无重复副作用能力已获全面验证 |
| 专用10步预算链、手动Compaction | 未到达；不以第一阶段预算停止替代全部原计划覆盖 |
| 满1M/多模态/任意中断/安全研究质量 | 本轮不覆盖 |

首个上游请求至停止/清理约161.47秒，含阶段间人工检查。不是持续30分钟稳定性通过。Provider原始usage累计：input345137、output16238、total361375；其中reported cached input250368。Full回放累计输入不等于独立上下文容量或计费金额；最大单次reported input46873。请求体从33592增至151420bytes，不据此断言缓存或工具定义需要裁剪。

## 阻断项 R6-01（优先修复）

观察链：

1. Request11恢复后发送当前Full；read_file/memory_read/inspect_project返回最新事实。
2. Request12已保留1个completed reasoning以及`project_snapshot_full + project_snapshot_diff`；当时24个sent Tools中没有read_output。
3. Request12返回精确`tool_search("read_output")`、另一搜索及样例命令；调用均已持久完成，read_output被当前Turn激活。
4. 新SamplingStepPlan的Tool集发生变化。`src/app/agent_loop.rs` 的candidate判断要求该理由允许state-diff，或candidate移除reasoning后与当前full logical input精确一致；后者在已有差量历史时不成立。
5. `src/domain/context/baseline.rs` 允许ToolsChanged的**精确**reasoning replay，但state-diff仅准入IncrementalInputUnsupported/ProviderRejected；因此这里明确停止，尚未产生第13份RequestAudit/网络请求。
6. RequestAudit的`validate_attempt_input_relationship`复用同一准入边界；不能只放宽AgentLoop分支而让审计契约失配。

不能泛化为“所有ToolsChanged都会失败”。当前证据是以上组合。已有`model_tool_search_updates_the_next_agent_loop_schema_and_resets_the_baseline`回归单独复跑1项通过，但使用增量能力且不覆盖本例的完整推理/差量交叉。首次命令因`--exact`未带模块前缀选中0项，已更正过滤后实际执行1项，不把0项当通过。

后续修复准入建议（尚未实现）：先在正式CLI fixture与App层复现该交叉，再明确什么样的Tool-only变化允许携带已经验证的差量/推理续接；同时验证精确sent Tool plan、配对顺序、ContextDeliveryMap、RequestAudit、预算和Turn/Resume失效边界。必须保留Prompt/Model/Capability/Core Memory等不兼容变化的拒绝测试。若改变冻结交付契约，先接受窄ADR/additive architecture；不得简单全放行ToolsChanged、静默丢推理、自动Resume/重跑工具或用厂商特例绕过。

## 非阻断观察，不与R6-01混修

- R6-02 工具发现可用性：`read_output durable blob stdout offset`返回空，精确工具名返回匹配。当前搜索按所有词都包含（AND）筛选；`stdout`等扩展词能排除正确工具。模型因此多搜索一次。后续用固定查询对评估，不直接加RAG/向量库。
- R6-03 可见范围表达：inspect_project的tool_capabilities来自当前resolved/sent plan，不是全注册表；模型把未列出的read_output误判为不存在。需要区分“当前已发送”与“可发现”，不另造工具目录权威。
- R6-04 非必要研究操作：2次Artifact登记因root transform/无有效TargetSnapshot来源被拒；未产生有效Artifact证据。模型还读取无关chain夹具、重复读Memory、显式inspect_project共3次，并为小任务写7405-byte报告。这些是成本/指导候选，不是证明哈希校验本身应删除。
- R6-05 模型自填时间：Memory/Snapshot调用中使用2025年时间而非本轮真实2026年环境时间。流水事件的运行时时间与模型提供观察字段需区分；本轮只记录，不重写事实或新增时钟框架。
- 测试限制：模型第一阶段已经读到后续chain内容和emit脚本文本，因此最终marker字符串本身不能证明原始回读/新发现。后续重跑必须以实际read_output调用和精确Blob字节为判据，或在阶段边界引入新随机marker；本轮没有据答案字符串声称覆盖。

## 后续边界

先锁定R6-01最小修复，复现→专项→受影响（含Port/依赖方向）→独立有限审计→修复→完整Gate→Lean。修复验收后再固定新版本、新Project/Session和明确剩余额度/新额度进行长测。未完成的大输出、副作用、Compaction覆盖保留；当前不继续消耗请求去绕开已定位阻断。

本轮只有外部验收脚本/夹具与本地状态/计划/报告文档变化；没有产品修复、新实现Gate或新的独立实现审计结论。基线的487项Gate是R5b4历史证据，不写成本轮重新执行。
