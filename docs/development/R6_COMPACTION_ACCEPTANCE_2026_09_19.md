# R6 压缩与压缩后恢复专项验收

## 结论

**压缩、请求准入、持久化和显式进程恢复路径通过；回答完整性验收有限。**
两次真实请求均HTTP200/completed，未执行新工具。首次回答却停在未完成标题
`## Prev`，CLI与持久History一致；没有保存上游原始响应体，不能确定是Provider
生成不完整还是Adapter解码遗漏，不得称为“只是显示问题”或直接改产品。
第二次为原计划的新进程Resume，不是重试首次回答；其回答正常结束并保留两种
标记、计数器和此前观察到的数值。R6不因此成为无限制的全项PASS。

## 范围与前置条件

- 2026-09-19用户明确授权补齐前置条件、进行压缩及压缩后恢复验证。新上限：
  8次实际请求、首请求起600秒；与上轮剩余20次分别计账。
- 使用上一轮真实Project/Session，不重跑工程修复、脚本或预算链：
  `/tmp/cyber-agent-r6-retest.9NyLBf/project`，
  `chat-1789746234-357960000-85193-1`。
- 新证据目录 `/tmp/cyber-agent-r6-compact.jSdtQW/`；变更前完整项目备份为
  `before-project/`，此前测试日志和结论没有重写。旧Project仍在原路径运行，
  没有通过复制到新路径来绕过Project身份检查。
- 同一已审计R6-01候选，locked build通过，源码/二进制哈希前后相同；二进制
  SHA256 `90f0a7d7f40742822924b399856f5fba771b6cc4be1ba5dde085584577590214`。
  本轮没有生产代码、永久测试、依赖、架构或配置Schema修改，无新实现Gate声明。
- 仓库外小型Rust设置程序重新链接当前库，通过已有TargetSnapshotDraft校验、
  ResearchReducer、Project Writer/EventSink追加一次真实工程目标快照。
  未手工编辑JSONL、扩大API、创建虚假Receipt/Finding或补造Artifact。
- 目标为`ledger.py`，实际SHA256
  `aecfc0230c79df724e12733c524173dddf2740a2c5f2a52de0d4992d24b04cd6`；
  快照`snapshot-r6-compact-ledger`。环境指纹来自当时OS/架构/Python观察，
  device字段明确只是本地OS/架构观察，不是唯一设备证明。时间取实际UTC。
- 外部设置程序初稿访问了私有查询方法，编译失败且未执行；改为使用公开事件
  读取和已有Reducer，不调整产品可见性。其后目标事件已成功追加，但Python
  校验器误读Research v2的`payload.data`层级而停止，零Provider请求。
  失败保留在`preflight-failure.json`；只修正外部校验器并显式验证已追加的
  唯一事件，没有重跑追加动作。Live前重新冻结控制器/二进制。

## 执行与验证

| 阶段 | 实际结果 |
|---|---|
| 零请求离线预检 | doctor ready；真实TargetSnapshot可重放；正式CLI `/compact`成功，首个模型请求尚未发生 |
| 压缩落盘 | 一个revision1事件，source History80项，retained40项；Summary v2引用真实目标，Research watermark3；原始152事件前缀不变 |
| 压缩后同进程采样 | Full/COMPACTION_RESET，HTTP200/completed，1次请求，0工具调用 |
| 新进程显式Resume | 同Session、同Compaction，Full请求成功，1次请求，0工具调用；实际审计reason为TOOLS_CHANGED，不声称观察到SESSION_RESUMED reason |
| 原始事实 | 原149个Blob字节不变，原工具Call/Result/Receipt和事件前缀未重写，counter仍为1 |
| 恢复内容 | 输出随机marker、chain marker、COMMIT_COUNT=1、fr-FR及0.22/7.04在第二次回答中保留；目标身份准确，未假称重新运行验证 |
| 回答限制 | 首次回答在`## Prev`截断；第二次回答承认该缺失。来源待诊断，不能证明上游完整响应与输出逐字一致 |
| 币种限制 | Memory中旧`Currency note: USD`未随settings改为EUR，压缩后模型将两者报告为冲突；不将货币结果当成精确复述通过 |

最终Session161个事件（新增9：两次Resume、一次Compaction、两条用户文本、
两次RequestAudit、两条助手文本），Research只追加一个目标快照。新144个审计
组件引用逐一检查大小/SHA256；两份wire请求配对合法、无previous_response_id。
正式Session show/export与最新prompt inspect均通过。CLI文本与新增持久助手
文本一致；本轮没有完整上游SSE原文，验证边界不包括端到端响应文本字节完整性。

Summary EvidenceSpine中旧Receipt引用数为0。快照是在既有工具执行后才建立，
原收据没有因此被追认成新目标的证据；旧收据和Blob均保留。不能把40项History
保留或有损摘要当成所有历史细节始终进入模型，也不能把引用数0误报成事实丢失。

本lane2次实际请求，合计90.89秒（含人工检查停顿）；Provider累计reported
input53027、output5888、cached input34944，最大单请求input26861。
不是600秒稳定性压力、1M容量、多模态或漏洞研究能力证明。

## 清理与后续

临时0600配置已删除；原Cyber Agent显式凭据源字节未变，证据目录精确密钥
扫描未命中。未访问Codex登录状态，未启动Actions、安全任务或Git提交/推送。
备份、原始失败记录和新增事实均保留，不自动回滚合法追加事件。

专项已补齐缺失的工程压缩/恢复覆盖。后续先单独诊断回答不完整：使用有界、
非敏感输入保留真实上游SSE，逐层对比delta、done/completed、Domain事件、
History和CLI；证明最小失败层后才修复。不根据本轮不足证据增加截断补丁或
额外续写请求。Memory备注与文件的冲突是夹具输入问题，后续应在测试准备时
显式消除或独立作为冲突场景，而不是修改本轮事实追认通过。

关键证据：`target-observation.json`、`offline-compaction.json`、`plan.json`、
`wire-observations.json`、`summary.json`、`verification.json`、`frozen-live.sha256`，
以及两个Chat进程stdout/stderr和完整before-project备份。未消耗额度不是自动
重跑许可；R6-02至05、跨Turn工具移除限制等不由此专项关闭。
