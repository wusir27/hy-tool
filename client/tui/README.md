# hy-tui

hy-tool 里的终端启动器（ratatui），不是第二套客户端：下载官方 `hy`、写 `~/.hy/client.yaml`，再 exec `hy client`。完整用法见 **[../tui.md](../tui.md)**。

日常使用请跑安装脚本（不需要 Rust）：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/client/install_tui.sh)
```

## 构建 / 运行

在本目录：

```bash
cargo run --release
```

或从仓库根目录：

```bash
cargo install --path client/tui
```

需要 Rust 1.85+。面向 macOS / Linux；没有 Windows TUN。

## 注意

- **Start TUN 需要 sudo**（macOS utun / Linux TUN）。hy-tui 保持普通用户，只对 `~/.hy/bin/hy` 走系统 sudo。密码不写磁盘。
- Run 速率是 TUN 网卡 ifstats（标 **TUN 出/入**），不是 QUIC/UDP 线字节。开了 `--route` 时计数含后来的 DIRECT/REJECT。缺网卡 / Stop 之后显示 `—`。
