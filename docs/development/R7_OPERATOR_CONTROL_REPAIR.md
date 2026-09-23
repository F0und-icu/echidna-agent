# R7-F2 — 用户纠偏与后台控制修复

## 范围与证据

用户报告：连接检查完成后擅自续做旧任务；后台模拟器 Job 使 tool_search
不可用。代码确认 CLI Turn 内不读输入，以及默认 GlobalSerial lease 会
挡住 catalog-only discovery。尚未用真实日志证明该次模型的全部行为归因。

复用 CLI cancellation/Turn/Resume、Base Prompt 和原 ToolRegistry；不建立
任务分类模型、Policy、目标数据库、通用消息队列或新调度器。只给 tool_search
窄豁免，不放开所有 ReadOnly 工具或设备冲突。stop/poll 通过原 discovery 路径。

## 代码预算

- 一个私有 CLI input 模块，Primary/initial Vision 两个生产消费者复用。
- Base v3 原字节移至历史 Fixture；v4 为唯一生产版本。
- 无新 pub API、Port、依赖、Event、配置、缓存或 Provider 分支。
- 一份新增架构/ADR；不修改旧冻结文件或 remote seal。

## 验收

专项测试 → 受影响测试（含 domain_ports_contract）→ 独立只读审计
→ 修复 → 完整 Gate → Lean。独立审计已单独获授权；Live/Git 不在本轮。
TTY 明确停止/纠偏、Vision、EOF、管道输入不误打断、恢复失败不自动提交；
后台 Job/PTY discovery/stop 可达且执行锁仍有效；Prompt/审计/历史兼容。
真实模型服从性未测；长驻服务进程树管理不在本次完成声明内。

## 实施记录

- 操作者已单独授权全新 Terra xhigh 限时只读审计，专项/受影响测试后派发。
- 首次 Job/PTY discovery-stop 回归通过；TTY 主模型停止/纠偏通过。
- 首次 Vision 夹具遗漏已有必需 TargetSnapshot，导致图像入库在请求前拒绝，
  测试等待 Vision 请求超时；补夹具 Snapshot，不放宽生产验证。原失败未算通过。
- 离线证据目录 `/private/tmp/cyber-agent-r7-f2.SVfJf3/`；本轮无 Live。
- 新 EOF 红测发现：终端 Ctrl+D 在活动 future 中被读取后，下一轮再次读取
  canonical stdin 会继续等待。通过同一 CLI 输入关闭标志传递修复；完成当前
  响应后退出，而不是取消它或再次等待。失败日志 focused-terminal-final.log 保留。
- 首版 Base 2032→2555 bytes（+523）使原 20000-window 压力测试 Required
  657 超过582tokens，受影响 CLI 86PASS/1FAIL/1ignored。已压缩新增文字，
  不提高原测试窗口、不削弱预算检查；旧失败见 affected.log / pressure-diagnostic.log。
  历史 v3 SHA256 `cc865337f9c30a27bea9e8d8b17653b7b1dd4a2582c9577c785b868637cfb17a`
  与 HEAD 原文件逐字节一致。start_job/start_pty/tool_search 更新至工具 version2，
  同一 canonical spec 进入审计、Fingerprint 与 dispatch；无重复契约。
- 精简后 Base 为2241bytes（净增209bytes）；原压力用例已通过，见 pressure-fixed.log。
- 受影响10suite共212PASS/1ignored；另有Job11、RequestAudit7，合计230独立
  用例通过/1ignored（离线测量 opt-in）。最终输入错误分支保留操作自身结果，
  不用 stdin 错误覆盖 App/持久化错误；全4TTY与2Port再检通过。
  完整CLI87PASS/1ignored在这个窄修复之前，完整Gate将在审计后覆盖最终版本。
- 最终审计候选为 candidate-review.sha256（14个当前source/test文件）和
  candidate-review.diff；早期候选清单与失败日志保留，不混作最终已审版本。
- 全新 GPT-5.6 Terra xhigh 于11:30:11UTC接收12分钟只读审计任务，限时内
  返回 APPROVE，无P1/P2发现、无要求修复项。独立核对14项候选hash，复跑4个
  TTY回归、Job/PTY发现停止、2个Port契约、既有unknown-effect/无虚假Receipt、
  Base与历史RequestAudit重建；结果见 independent-audit.md。实现者未自审批准。
  完整Gate正在运行；此处不把独立专项复跑称为完整Gate或Live。
- 首次完整Gate在既有 full_reasoning_with_project_diff_survives_additive_tool_activation
  用例的15秒子进程退出期限失败（CLI86PASS/1FAIL/1ignored），见 full-gate.log。
  未改代码、超时或断言，精确单例复跑8.72秒通过，见 gate-failure-isolated.log。
  这支持时序敏感性嫌疑，但不足以证明确切根因；保留首次失败，不以单例通过替代
  完整Gate。相同命令、原默认并发的全Gate复跑见 full-gate-rerun.log。
- 独立审计者另行6分钟限时复核：原候选APPROVE仍有效，不代表Gate通过；
  14项hash未变、无新增P1/P2。该用例用管道输入，不走新增TTY读入；原测试
  timeout分支丢弃子进程输出，因此无法定位具体停滞原因。保留测试诊断缺口，
  不把单例复跑或审计批准当作根因已解决；增强旧harness超时证据另列后续项。

## 最终本地 Gate

相同命令、默认并发完整复跑 exit0：fmt、locked全target/feature Clippy（warnings
denied）、45个集成suite共532独立PASS/4默认ignored、locked build、src inline
扫描、git diff --check。原始533次PASS包含writer-lock子进程的1次重复；两个
Live Smoke及两个离线测量按原规则ignored。CLI87PASS/1ignored，先前超时用例
此次通过；首次失败仍保留，不声称根因已修复。
14项候选hash、325项完整source/test hash与已审tracked diff一致，无审计后代码改动。
本地Gate不是远端CI或Live；架构v1.22仅本地验证，不做remote hash seal。

## Lean Review — PASS

1. 唯一新增所有者为私有 `cli_chat/input`（72行），Primary与initial Vision复用。
   它仅观察终端/排队一行，复用App取消与正常CLI边界，不复制取消/Resume/持久化。
2. 无第二Capability、错误映射、配对、预算、排序、Fingerprint或取消实现。
3. 无新增pub API、Port、依赖、Event、Config、Schema或Cache。Base generation4
   由原Context路径消费；三项工具version2由原canonical spec/plan路径消费。
4. 无测试专用生产入口、未来占位或为绕过失败创建的抽象。
5. 四项TTY回归及一项Job/PTY场景永久保留于tests；src inline扫描通过。
6. 本次是本地REIMPLEMENT，无上游复制/PORT或Ledger新增。
7. Cargo/依赖未改，无codex/路径依赖；既有domain_ports_contract通过。
8. 删除两段重复Ctrl+C等待，生产v3移至历史fixture，精简首版过长v4；
   无新增依赖/装配点/第二调度器。生产Rust净增102行。服务进程树管理、
   任务分类器、自动重试、真实模型质量验证继续Deferred。
9. 单Base人格，Domain/Phase/Profile仍正交，无任务状态机或runtime多Agent。
10. Base版本及工具canonical版本经原RequestAudit/Fingerprint/计划路径交付；
    旧v3字节hash与历史审计重建已验证，不重写历史。
11. Job/PTY说明明确生命周期、资源和停止；无自动重试、虚假Receipt或回滚承诺。
12. 精确ResolvedToolPlan/Hidden/后续Step激活/下一Turn过期保持，回归验证。
13. Context sidecar、Provider身份、Summary与事实权威不变。
14. 无Shadow/RAG/算法推广，不以Prompt规则宣称模型服从性或成本改善。
15. 顺序严格为专项→受影响→独立审计→修复（无需）→完整Gate→本Lean；
    Gate首次失败保留并复核，同代码完整复跑通过，不放宽门禁。
16. 独立Terra xhigh在12分钟内APPROVE；追加6分钟只读复核保持候选批准。
    实现者不是审计者，未把无回复/超时或单例通过算完整Gate通过。
17. 私有helper有两个真实生产消费者；无test-only公共API、无消费者类型/占位。

剩余治理债务：既有15秒子进程超时harness丢弃输出，应独立补足超时诊断；
本轮未改它，单次失败原因未定。真实模型是否遵守“检查后停止”需操作者实测。

## 操作者测试交付

独立入口 `/Users/f0und/2026/cyber-agent-manual-test.WQf6SD/run-r7-f2.sh`
使用单独固定的验收二进制，旧入口/二进制/配置/项目/日志原样保留。
不指定Resume时新建Session，但同Project记忆仍复用，不称为空白项目。
仅准备入口，未运行Live；sha与使用说明记录在该目录R7-F2.md。
