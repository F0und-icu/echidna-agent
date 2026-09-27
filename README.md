# Echidna

**Local Vulnerability Research Agent** · Question. Observe. Verify.

本地优先、单用户的漏洞研究 Agent。一个 Rust package、一个主 AgentLoop，保留
可追溯的工具执行、研究证据与显式 Session 恢复。Echidna 是后续开发主仓库；
[cyber-agent](https://github.com/F0und-icu/cyber-agent) 保留为 core 历史基线。

<img src="docs/design/echidna-mark.svg" alt="Echidna serpent E" width="100">

## 本地使用

```sh
cargo build --locked
./target/debug/echidna --help
./target/debug/echidna --version
```

在独立研究目录中用上述可执行文件的绝对路径运行 `init`，然后运行
`--config /absolute/path/provider.toml doctor` 检查配置；准备使用真实 Provider
时运行 `--config /absolute/path/provider.toml chat`。配置见
[开发环境与配置](docs/development/DEVELOPMENT_ENVIRONMENT.md#6-配置和目录)。
`doctor` 不发模型请求；`chat` 提交任务后会使用所配置 Provider。

CLI 回复标签为 `Echidna> `。终端有简洁品牌欢迎；`NO_COLOR=1` 关闭主题颜色，
管道中不增加品牌欢迎或 ANSI。当前仍为交互式 CLI，尚无机器 worker 协议或 TUI。

## 数据与构建兼容

现有 `.cyber-agent/` 项目目录和 `CYBER_AGENT_*` 配置/凭据变量继续使用。
事件、Prompt 与历史数据不因更名而迁移。Rust 外部调用者需将 crate import 改为
`echidna`；不提供第二个旧名 binary。`-V` 为版本，`--version` 附构建时 HEAD；
HEAD 不等价于干净源码校验，完整性以候选记录及分发 SHA256 为准。

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked
./scripts/package-macos-arm64.sh
./tests/distribution_package.sh dist
```

默认验证使用离线 fixture/loopback；Live 另行显式授权。当前新仓库 Actions 关闭。
工程测试通过不代表模型始终遵守范围，也不代表完整长任务或全部窗口容量已验收。

## 项目入口

- [当前状态与验收边界](docs/development/STATUS.md)
- [迁移与回滚记录](docs/development/ECHIDNA_MIGRATION.md)
- [主题及命名](docs/design/ECHIDNA_IDENTITY.md) · [主题样张](docs/design/theme-preview.html)
- [随模型发展的研究设计](docs/design/FUTURE_RESEARCH_DESIGN.md)
- [外部并行协调设计](docs/design/EXTERNAL_ORCHESTRATION.md)
- [完整文档索引](docs/README.md)

未来设计只作储备。历史文件中的 Cyber Agent 是继承内核的名称，冻结架构及验收
记录保持原样。Echidna 与其他同名项目及所参考作品没有官方关联。
