# Echidna 仓库迁移与回滚

日期：2026-09-27。用户决定让新仓库成为后续开发主线，旧仓库保留为 core，
并明确要求旧仓库提交最后一批修改。不是在原仓库改名，也不是建立薄 UI wrapper。

## 基线

- 旧库：<https://github.com/F0und-icu/cyber-agent>。
- 旧工作分支：`codex/r7-f1-long-task-repair`。
- 最后提交：`eee836d056bf71a008c5d29d0bf94c158809e687`，本地/远端 SHA 一致。
- 该提交保存 R7-C1a 源码、永久测试、ADR/v1.24、结果和 NET-01 设计记录。
  原 `main` 与 Draft PR #88 未合并；“core 基线”不表示新一轮稳定性验收或架构封存。
- 既有独立审计/Gate 对应334源码/测试文件，迁移前全部 SHA256 重验一致。
  6份历史 Prompt 迁移 fixture 与原始字节一致；现有工作成果没有重写。

新私有库：<https://github.com/F0und-icu/echidna-agent>。
从旧本地库独立 clone（无 hardlink），保留 HEAD 的完整祖先历史；没有复制旧 GitHub
PR/Issue、远端设置或旧 release tags。新 `main` 和 `codex/pre-echidna-rename`
最初均指向上述 core 提交，远端回读 SHA 一致后才开始品牌修改。
后者长期保留为更名前快照，不随开发推进。

全部446个版本文件与原工作树快照逐字节对应。`codex/`、`target/`、真实 Provider
配置、`.env`、`.cyber-agent/` 与本地研究产物均未复制。旧仓库提交后保持干净；
新开发目录为 `/Users/f0und/2026/echidna-agent`。

## E0 边界

包/crate/binary、CLI 命名与呈现、构建标识、打包命名、当前文档入口迁移。
保留 `.cyber-agent/`、`CYBER_AGENT_*`、事件 family、工具 wire 身份、Prompt 以及
所有冻结架构/ADR。旧资料中的 cyber-agent 名称指继承基线，不批量替换历史 URL。
无新 API/依赖/schema；core 暂不作为 Git dependency、submodule 或独立 SDK 接入。
关于内核的后续改进先在 Echidna 主线进行，提取独立 core 要等实际消费者与单独设计。

E0 先在 `codex/e0-echidna-identity` 完成验证，再以快进方式更新新库 main；这是
新项目初始化交付，不合并旧库 PR、不改写其历史。所有提交带 `[skip ci]`；新库
Actions 在第一次 push 前已关闭。没有运行 Live、创建 tag/release 或替换旧测试 binary。

## 回滚方式

在新的独立目录 checkout `codex/pre-echidna-rename` 即可重建原名程序，或从旧库
上述提交重建。保留新开发分支，不用 reset/force-push 删除历史。回滚品牌提交时
可创建 revert 提交，经验证后正常推送。

这是源码回滚，不是数据回滚。旧于 R7-F3/C1a 的 binary 可能无法 Resume 包含新拒绝
事件/Phase generation 的 Session；不要拿更老程序直接打开新会话。E0 保留数据契约
不等于对所有未来版本作无限兼容承诺。研究数据备份由其所在 Project 单独管理。

## 验证记录

本次证据目录：`/tmp/echidna-migration-7yvma8u2/`。迁移前原文件manifest、Git
状态、专项/受影响/完整 Gate 日志及独立审计记录保存在此。验收结果以 STATUS 为准；
审计前不宣称通过，离线模型脚本不作为真实模型服从性证据。

E0 独立审计、完整离线 Gate 与 Lean 已完成，详细次数、首轮超时与后续复测、
构建产物的来源限制见 [E0验收记录](E0_ECHIDNA_IDENTITY_VERIFICATION.md)。

发布回读：代码提交 `4f7ecb7`、设计/验证提交
`d30a5dc86f66557d0fa2408a2897079ee56ebb1e` 已推送至新库 main 与 E0 分支，
本地/远端 SHA 一致。更名前快照仍为 `eee836d`；新仓库私有、默认 main、Actions
关闭且运行列表为空。旧仓库仍干净，main 未更新，原 Draft PR 未合并。
