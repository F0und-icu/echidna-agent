# R7-F3 — 非法工具参数的有界重新生成

## 授权与范围

操作者确认最多2次模型纠正、成功后继续原任务。仅本地实现与离线验证；
操作者现已授权全新GPT-5.6 Terra xhigh独立只读审计，限时12分钟；
审计后修复，再进行完整Gate与Lean。无Live、Git、Actions、凭据操作。
保留未提交R7-F2，不覆盖旧测试入口、二进制、项目或日志。

现有拒绝行为避免误执行，却让一次模型JSON格式错误直接停止本轮。
本次只放开typed执行前错误的重新采样，不把普通断流/执行错误归为可重试。
ADR-0033/v1.23明确替代旧的这项限制，旧冻结文档/失败证据不改写。

## 单一所有者

- Responses仍严格解析完整arguments；中性诊断仅类别、行列、字节数。
- AgentLoop独占每Turn累计纠正计数，最多2次；中途成功不清零。
- 每次请求仍由原Primary/Vision共享预算准入，原RequestAudit先落盘。
- CLI现有持久化回调把新version1 `model.sample_rejected`写入同一Session，
  绑定失败请求的精确audit Event ID。Reducer拒绝错引用、重复或越界，
  不生成成功History、ToolResult、Receipt或研究事实；写入失败立即停止。
- 私有argument_correction构造Required运行时反馈，ContextPlanner唯一负责
  预算、渲染和provenance。明示非用户输入，SessionProjection/DerivedNonEvidence，
  不进Conversation History。成功交付的反馈按原continuation保留，追加不重复。
- 保留失败前已提交reasoning、续接ID、执行结果；不使用失败响应ID、不自动Resume。
- 下轮清空纠正额度/逻辑反馈；显式Resume不重投旧反馈。保留的Provider交付前缀
  可能仍有历史反馈，这是精确续接，不是新指令或新的证据权威。

## 代码预算与兼容性

新增一个私有App模块；无新依赖、配置、Port方法、Provider分支、缓存、
通用重试框架或自动JSON修补。新增中性诊断/错误变体与Session事件均有真实
生产消费者；不新增测试专用pub入口。新事件使旧二进制对含它的Session严格
拒绝读取，不保证旧版能Resume新版会话；旧会话由新版兼容读取。
原固定测试入口保留不会自动升级，更不能用旧二进制去恢复新事件会话。

## 离线验证记录（持续更新）

证据目录 `/private/tmp/cyber-agent-r7-f3.oSh33u/`。

- 初次check发现Compaction调用Context编译时遗漏新增反馈参数；为显式Compaction
  传空slice修正。未改变Compaction契约，随后all-target check通过。
- focused-regeneration-expanded.log：5个CLI回归通过（含Full-only/增量矩阵），
  覆盖历史效果仅一次、失败兄弟调用零执行、纠正上限不因中途成功重置、预算耗尽、
  显式Resume、拒绝事件持久化失败及strict重放/篡改负例。
- focused-loop.log：3个回归通过，覆盖纠正前审计失败/取消、其他错误不重试、
  首步失败后仍保留上一Turn精确激活过期证明。
- focused-decoder.log与focused-atomicity.log：各1项通过，三类非法JSON均严格
  拒绝整次采样，metadata-only/redaction及2次后停止保留，审计可重建。
- affected-contracts.log：14个受影响套件205PASS/2默认ignored，含
  domain_ports_contract、Session/Context/Tool Loop/Responses/Replay/Vision等。
- affected-cli.log：完整CLI92PASS/1默认ignored，合计15套件297PASS/3ignored；
  专项测试已包含于这些套件，不重复累计。三个ignored均保持原有显式启用边界。
- focused-clippy.log：lib/bin及三个直接受影响测试目标的warnings-denied Clippy通过；
  fmt、git diff --check及src inline-test扫描通过。这是审计前验证，不是完整Gate。
- 独立审计：全新GPT-5.6 Terra xhigh在12分钟内返回APPROVE，无P1/P2问题；
  独立13PASS，328项源码/测试hash及tracked diff审计前后相符。
  详见同目录independent-audit.md。无须源码修复，随后开始完整Gate。
- 非阻塞覆盖建议：独立增加“增量fallback后非法参数”的精确audit引用用例，
  以及纠正时Core Memory/Prompt改变用例。审计已检查现有守卫，未发现缺陷；
  这些组合的专门新fixture仍Deferred，不冒充已经逐项动态覆盖。

## 完整 Gate 与 Lean Review

独立审计后未修改源码/测试。完整Gate脚本run-full-gate.sh及full-gate.log
保存在证据目录；exit0，fmt、locked all-target/all-feature warnings-denied
Clippy、完整测试、locked build、inline/whitespace扫描全部通过。
45集成套件540独立PASS/4ignored；原始541PASS包含既有writer-lock子进程
测试重复报告。4ignored是2个显式Live Smoke和2个显式离线测量，未启用。
Gate前后328项源码/测试hash均匹配独立审计冻结。本轮Gate无失败、无超时放宽。

随后Lean Review PASS（主Agent治理复核，不冒充独立代码审计）：

1. 新语义只有typed参数拒绝后的有界模型纠正；计数/决策由原AgentLoop拥有，
   私有argument_correction仅构造反馈，Session原Reducer拥有拒绝事件验证。
2. 无第二份Capability/配对/错误解析/预算/取消/排序/Fingerprint实现。
3. 中性diagnostic/category由Responses生产、Port/App/CLI/Session消费；
   新错误由Adapter生产、AgentLoop消费；拒绝checkpoint由App生产、CLI持久化；
   Session事件由原Writer/Reducer消费。无新依赖、配置、缓存或Port方法。
4. 无测试专用接口、未来占位、通用重试/Hook/Provider路由框架。
5. 永久回归留在tests/；src inline扫描通过，未删除回归保护。
6. 本地REIMPLEMENT，没有复制Codex代码，无新增PORT或许可证依赖。
7. Cargo文件/锁文件/工具链无变动，Port依赖检查通过；不依赖本地codex/。
8. 删除旧的这一类generic ProtocolFailure映射，改用唯一typed诊断；
   不保留猜修JSON、Adapter重试或临时实验实现。两项组合fixture明确Deferred。
9. 未增加人格、角色Agent、Phase或状态机；仍是单AgentLoop。
10. 反馈走原ContextPlanner/RequestAudit/manifest路径，可重建实际请求；
    没有旁路Prompt或另立日志权威。
11. 无Tool Schema变更、执行重试或虚假Receipt；同失败采样的合法兄弟调用
    也不执行，已完成效果保持一次。真实模型是否重复提出动作仍未由Live证明。
12. 每次纠正重新捕获原不可变SamplingStepPlan；首个成功采样前仍保留
    前Turn激活过期证明，不把旧Tool重新开放。
13. feedback sidecar明示SessionProjection/DerivedNonEvidence/Ephemeral；
    protocol user表示不等于用户History，成功交付后按原续接保留精确前缀。
14. 本轮不修改Shadow/检索，不宣称Token、延迟或模型能力收益。
15. 顺序完整：专项→受影响/Port→独立审计→修复（无要求项）→完整Gate→Lean。
16. 全新Terra xhigh有限范围只读审计，在12分钟内有明确APPROVE；未用沉默替代。
17. 新pub契约均有上述生产消费者，无仅为测试扩大可见性或controlled入口。

生产Rust相对incoming R7-F2新增284行、删除8行，净增276行；含唯一新增
42行私有App模块。CLI重叠部分按已审R7-F2 candidate-final.diff扣除，不把
R7-F2算成本轮新增；未删除模块。运行时事实新增仅同Session内一个version1事件，
不是第二审计源。旧模块超过300行的拆分信号已审查：本次抽出独立反馈构造，
预算/续接/Reducer仍留原所有者，未为行数拆出第二控制器。

## 验收边界与保留限制

当前COMPLETE_LOCAL_OFFLINE_VERIFIED，停在本地交付。不自动启动下一Slice。
所有目前验证均为离线Mock/loopback；没有真实模型格式修正率、费用/Token节省
或安全任务成功率结论。此次没有替换操作者现有固定测试程序或启动Live。
旧R7-F2的15秒测试超时诊断缺口仍独立保留，本轮没有顺手改harness期限。
