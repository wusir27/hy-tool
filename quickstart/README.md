# hy 快速上手（0→1）

产品：[hy v0.0.1](https://github.com/wusir27/hy/releases/tag/v0.0.1) — [Hysteria 2](https://github.com/apernet/hysteria)（协议 v4）的 Rust 版。二进制名：`hy`。

按下面顺序做完，你会得到：

- 一台 **Linux 服务端**：`hy-server.service` 监听 **UDP 443**
- 一个 **客户端**：本机 SOCKS5（`127.0.0.1:1080`）；可选 HTTP 代理；macOS 还可再开 utun 整机走隧道

## 阅读顺序

1. [server.md](server.md) — Linux 服务端安装、证书、示例配置、启动、防火墙
2. [client.md](client.md) — 下载同版本客户端、配置、测试
3. （可选）[macos-utun.md](macos-utun.md) — 在 SOCKS5 之外做 macOS 系统级隧道

示例 YAML（可直接拷）：

- [examples/server.yaml](examples/server.yaml)
- [examples/client.yaml](examples/client.yaml)
- [examples/client-macos-utun.yaml](examples/client-macos-utun.yaml)

安装脚本与路径对照：[install/](../install/)。完整字段与场景：[USAGE.md](https://github.com/wusir27/hy/blob/main/USAGE.md)。
