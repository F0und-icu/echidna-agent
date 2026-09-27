# Echidna 产品身份

状态：E0 实施范围。产品为 **Echidna**，仓库为 `echidna-agent`，Rust package/crate
与默认可执行程序为 `echidna`。副标题：**Local Vulnerability Research Agent**。
原创短句：**Question. Observe. Verify.**

## 主题

古希腊神话中的 Echidna 提供蛇形、古老与未知的视觉线索；Re:Zero 的艾姬多娜
提供求知欲、冷静、茶会、黑白与淡紫的气质。它们指导产品呈现，不改变漏洞研究
事实纪律，不要求模型扮演角色。保持唯一 Pragmatic Vulnerability Researcher
Base 身份，Base v5 和各 Phase 原字节不因品牌更改。

- 主标记：原创 E/蛇形线条；茶杯只作次要视觉线索。
- 色彩：墨黑 `#19171F`、象牙白 `#ECE6DB`、冷淡紫 `#A697C3`。
- 标题可用衬线字，研究内容和终端以可读性为先。
- 不复制官方立绘、徽记、台词，不声称与作品或其他同名项目关联。

[主题样张](theme-preview.html) 与 [原创 SVG](echidna-mark.svg) 是可独立查看的源码资产。
本阶段终端只采用一行品牌欢迎、一行短句和 `Echidna> ` 回复标签。
欢迎要求 stdin/stdout 均为终端；颜色还要求没有 NO_COLOR，且 TERM 不是 dumb。
终端使用标准 magenta 槽，具体色值由用户终端主题决定；精确紫色仅用于样张。
管道输出保留原 CLI 文本结构，去掉主题欢迎与新增 ANSI。它还不是 headless 协议。

## 命名与兼容

`echidna -V` 输出简短软件版本，`echidna --version` 附源码基线 commit。
基线 commit 只表示构建时 HEAD，不保证工作树干净或全部字节等于提交；无 Git
的源码包明确显示 unknown。产物完整性另由打包 SHA256 和候选 manifest 核对。

| 对象 | E0 处理 |
| --- | --- |
| 产品/CLI 帮助/默认 binary/crate | Echidna / echidna |
| 回复标签 | Echidna>，不写入 History 或模型请求 |
| 既有数据目录 | 保留 `.cyber-agent/`；不自动复制、改名或迁移 |
| 配置/凭据/Live 变量 | 保留 `CYBER_AGENT_*`，无新兼容别名 |
| Event family / 工具返回中的内核身份 / fingerprint 标记 | 保留既有 cyber-agent 协议标识 |
| 历史文档、Prompt、fixture、架构、ADR | 原字节保留；当前入口解释名称对应关系 |

新仓库作为后续完整产品开发主线。旧仓库是可回溯 core 基线，目前不是发布的
独立 SDK，也没有自动同步关系。不要先加 wrapper、复制第二套 Session 所有者
或扩大公共 API；出现真实第二个前端消费者时，再单独设计 core 提取契约。
仓库名加 `-agent` 区分现有同名工具（如 crytic/echidna）；品牌不代表名称独占。
