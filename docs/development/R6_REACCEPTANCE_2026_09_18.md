# R6-01 修复后普通工程长测

## 结论

**R6-01 真实模型复验通过；R6 整体验收未完成。** 本轮在手动 `/compact`
阶段因夹具未建立当前 Research TargetSnapshot 而停止。该前置条件来自现有
实现和已有回归，不能归类为新的推理续接故障，也不能将未执行的压缩/压缩后
恢复算作通过。验收准备遗漏由本轮控制器负责；没有放宽产品规则或热改代码。

## 固定版本与边界

- 用户在 R6-01 本地修复交付后明确要求“开始长测”。新额度为最多1800秒、
  累计48次实际上游请求（含恢复），不是沿用首轮未用额度。
- main `5c073086bb41cae048dcb67acbe0e4a47d711816` 加未提交 R6-01 候选；
  该候选已独立 Terra xhigh APPROVE，完整 Gate491项通过/4项忽略。
  本轮重新 locked build 成功，没有重跑完整 Gate 或修改产品源码。
- 开发二进制 SHA256：
  `90f0a7d7f40742822924b399856f5fba771b6cc4be1ba5dde085584577590214`。
  运行前后源码、二进制和控制器哈希均一致。
- 正式交互 CLI/Config v1，Primary `deepseek-v4-flash`、Responses Full-only、
  Vision disabled、声明窗口1048576、每Turn准入8（受控压力，非默认值变更）。
- 全新 Project/Session：`chat-1789746234-357960000-85193-1`，贯穿5个Chat进程。
  日志与事实：`/tmp/cyber-agent-r6-retest.9NyLBf/`。
- 复用仓库外 TLS 验证的转发计数器，所有语义请求体原样转发、无重试；
  它是测试条件，不是新增产品网络层或无转发性能基线。
- 没有安全题目、Docker、Actions、Git提交/推送、Codex登录状态访问。
  原显式 Cyber Agent 凭据源字节未变；临时0600配置已删除，结束时精确凭据
  扫描未在保留证据中命中。日志、密钥和真实Session不进入Git。

## 实际结果

28次实际请求，全部HTTP200并观察到completed；307.56秒（含阶段间检查），
不是持续30分钟稳定性证明。各阶段请求数为8、2、7、8、3。

| 验收项 | 结果与证据 |
|---|---|
| 普通工程修复 | 初始7项测试含16个失败断言；修复后7项固定测试和5项外部oracle通过，固定输入未改 |
| 环境与执行 | 首先inspect_environment；观察Python3.12.4/macOS，未发现GNU参数误用 |
| 第一次预算停止/恢复 | 8次后真实检查点停止，新进程显式Resume用2次完成，无自动请求追加 |
| 文件/Memory新鲜度 | 退出后settings USD→EUR、Memory en-US→fr-FR等长修改并恢复mtime；实际读取及回答使用新值 |
| 当前结果 | USD alice20.01；EUR alice0.22/bob7.04，与外部oracle一致 |
| R6-01组合路径 | Request16为TOOLS_CHANGED Full：旧ToolSpecs精确保留、新增Tool、completed reasoning和project_snapshot_diff同时存在；实际HTTP200继续 |
| 一次性副作用 | 一次serial run_commands依次执行commit_once.py、emit_output.py；各出现一次，最终计数器1，后续Resume未重跑 |
| 子命令与大输出 | stdout173985字节，完整Blob经SHA256/大小验证；运行时UUID标记位于87000偏移，实际read_output从86800附近读取后报告正确 |
| 10步预算链 | 夹具在此阶段才生成；每响应一次read_file，前8步后预算停止，新进程只读取9/10，不重复已完成读操作 |
| 手动Compaction | 未执行：当前Research流没有TargetSnapshot，CLI明确拒绝；无compaction Event、无第29次请求、后续总结输入未执行 |
| 全1M/多模态/任意中断/安全研究质量 | 本轮不覆盖 |

中段输出标记为运行时随机生成，仅存在于实际捕获输出；预算链也未提前放入
项目。通过依据是持久调用、精确Blob/偏移和结果配对，不是模型猜中答案。
一次batch本身不保证永远无法重发；本轮仅证明观测到的执行与恢复没有重复。

最终152个Session Events、30 Call/30 Result/30 Receipt，全部工具成功；
7个子命令检查点。校验2130个审计组件引用、18个捕获引用，涉及145个唯一Blob。
28个wire请求全部Full且配对顺序合法；History无reasoning；正式Session show/export
和最新prompt inspect通过。校验脚本没有另写完整协议重编码器，也没有对每个
历史manifest重算全部BLAKE3关系；不夸大独立验证范围。

Provider累计reported usage：input1156272、output20793、total1177065，
cached input980608。最大单请求input52887；请求体33592至186300字节。
累计输入超过1M源于多次Full请求，不是满1M窗口能力或实际费用证明。

## 压缩停止的原因

`src/adapters/research_projection.rs::compose_session_summary_draft` 要求
`latest_target_snapshot_id`，缺失时返回 `CompactionCurrentTargetRequired`。
`src/cli_chat/compaction.rs` 在追加事件前组装/验证候选；既有
`m5_compaction_preconditions_never_append_a_partial_fact` 明确覆盖这一拒绝。

本轮Research流只有两条Memory外部变化事件，没有记录目标快照。项目文件扫描
的 ProjectSnapshot 不是 Research TargetSnapshot，不能用前者代替后者。
普通工程任务不需要伪造漏洞Finding，但压缩验收必须预先建立真实、可验证的
工程目标快照。控制器没有检查这一前提，是验收夹具缺口。

不修改本轮Session来追认通过，不续用剩余20请求进行隐式重跑。后续可单独锁定
有真实TargetSnapshot前置条件的有界压缩/压缩后恢复专项，不必重测已通过部分。
是否让无Research目标的普通聊天也能压缩，是另一个设计议题；本轮不改契约。

## 保留问题

- R6-02复现：`read_output durable blob stdout`搜索为空，改用精确名称才命中；
  增加了一次模型采样，不引入RAG或新工具目录来绕过。
- 多次首次操作携带`execution.rerun_reason`，包括预算链首次读取；未造成实际
  重复执行，但后续可检查工具说明是否让模型误解该可选字段。
- 跨Turn工具移除与保留差量推理的已知限制不由本轮解除；本lane各阶段采用
  显式进程Resume，不能据此宣称同进程任意Turn无缝连续。
- 旧R6-03至05未在本轮全面重新评估；未覆盖不等于已修复。

关键证据：`plan.json`、`source.sha256`、`harness.sha256`、`phase-results.json`、
`wire-observations.json`、`summary.json`、`failure.json`、`facts-verification.json`、
`coverage-stopped.json`、`oracle-summary.json`及分阶段stdout/stderr。验证脚本在
仓库外，只读产品事实；控制器/模型输入在首个请求前冻结，未边测边改。
