# hy 快速上手（0→1）

1. 按 [server.md](server.md) 在 Linux 上安装、配证书、写 YAML、开防火墙并启动服务端。
2. 按 [client.md](client.md) 下载同版本客户端、改配置、跑起来并用 curl 测 SOCKS5。
3. （可选）按 [macos-utun.md](macos-utun.md) 在 macOS 上再开 utun，让系统流量进隧道。

> 产品：[hy v0.0.1](https://github.com/wusir27/hy/releases/tag/v0.0.1) — [Hysteria 2](https://github.com/apernet/hysteria)（协议 v4）的 Rust 版。二进制名：`hy`。
>
> 做完后：一台 Linux 服务端（`hy-server.service` 监听 **UDP 443**）；一个客户端（本机 SOCKS5 `127.0.0.1:1080`；可选 HTTP 代理；macOS 还可开 utun）。
>
> 示例 YAML：[examples/server.yaml](examples/server.yaml)、[examples/client.yaml](examples/client.yaml)、[examples/client-macos-utun.yaml](examples/client-macos-utun.yaml)。
>
> 安装脚本与路径对照：[install/](../install/)。完整字段与场景：[USAGE.md](https://github.com/wusir27/hy/blob/main/USAGE.md)。
