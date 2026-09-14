# 普通工程场景测试报告 — 2026-09-14

## 结论与范围

PASS。本轮未发现产品故障；没有修改生产代码、依赖、Prompt 或运行配置契约。
安全/CTF/PWN 实测由操作者负责，本轮模型只执行普通 CSV 记账任务。

- 基线：main `3f880f99b52df087c272e29c9dc11edca12d9a85`（R5a / PR#79）。
- 实际二进制 SHA256：`cdd358bb869cae15e82294e145f5fcb5ccff44a426a4c31e5d25664bacdf6b6c`。
- 环境：macOS15.7.5 / x86_64；正式 CLI、唯一配置装配路径。
- 模型：显式配置的 `deepseek-v4-flash`，Responses Full-only，Vision disabled，context_window1048576。
- 限额：剩余 L1-3，一个新项目/Session，3个 operator Turns，每 Turn 最多2次准入，总上限6次请求/300秒；没有重试或扩额。

## 离线验证

现有7个套件，63项通过，0失败、0忽略，命令 exit0：

```sh
cargo test --locked --test memory_snapshot --test context_planner \
  --test session_resume --test session_compaction --test background_jobs \
  --test pty_sessions --test domain_ports_contract -- --quiet --test-threads=2
```

覆盖 Memory/Snapshot 缓存和外部编辑、上下文预算、Session/压缩与输出引用、Job/PTY 生命周期、依赖方向。某些永久回归名称带历史 CTF 前缀，但只运行本地合成夹具，不调用真实模型解决安全题目。本轮没有重跑完整 Gate；未改产品，前一 R5a 完整 Gate 的证据独立保留。

## 真实模型结果

三阶段共17.299秒，只有3次实际 `read_file` 工具调用，没有模型发起的 Shell、网络工具或非指定文件读取。

| 阶段 | 测试操作 | 最终持久回复 | 累计审计/收据 | CLI耗时 |
|---|---|---|---|---|
| 1 | 首次读取合成 CSV；Core Memory=USD | `TOTAL=70 CURRENCY=USD` | 2 / 1 | 6.061s |
| 2 | 进程退出后修改 CSV，显式恢复同 Session | `TOTAL=80 CURRENCY=USD` | 4 / 2 | 4.208s |
| 3 | 进程退出后改 Memory=EUR，再次显式恢复 | `TOTAL=80 CURRENCY=EUR` | 6 / 3 | 6.536s |

CSV 和 Memory 修改前后长度相同，并恢复、核验原 mtime。每阶段同时核对：实际 ToolResult 内容、未截断成功状态、Call/Result 配对、精确耐久 Assistant 答案、全部 RequestAudit 的 Full 模式、严格 Session 回放和 prompt inspect。旧 Session 文件前缀保持不变。Snapshot 文件清单未纳入外置测试日志。

## 审计与隔离

外部测试脚本经独立 Luna xhigh 有限审查：首次 REQUEST_CHANGES，补强非 Live 预检不读取凭据、持久结果/Full 校验、0600权限断言和清理记录后，复审 APPROVE。未通过审查前没有运行 Live。

0600临时 Config 已删除；原始 Cyber Agent 凭据输入未改，保留日志/事实未发现该凭据。Codex 登录状态未读取或修改。没有 Actions、Docker、目标题目、WP/EXP、产品代码修改或本轮远端提交。

## 限制与待办

- 审计/准入次数不是直接抓取的网络请求数；Provider usage、TTFT、内部 hash 次数/耗时、扫描次数与运行缓存命中未测量，不报告为0。
- Session 文件累计28889→64167→106330 bytes；这是这个短场景的持久文件大小，不是 wire bytes、Token 或性能瓶颈证明。
- 这是进程正常退出后的恢复测试，不是新增的崩溃中断、长时间负载或满1M容量验收。记忆采用正确也不代表通用记忆质量已测量。
- R5b 四组固定测量和优化仍待完成；没有基于这次结果新增缓存或宣称速度优势。R6安全研究能力由操作者实测。
- 原三次 L1 合成 Session 额度已全部使用。后续可另定普通工程长任务的请求/时间预算，不自动借用旧 PWN 额度。

完整外部证据：`/tmp/cyber-agent-general-test.UBjhge/`，包含 `offline.log`、`live_csv.py`、`plan.json`、`phases.json`、`summary.json` 和三个阶段的 stdout/stderr/Session/Prompt 检查。测试脚本不进入产品源码；日志和真实配置不进入 Git。
