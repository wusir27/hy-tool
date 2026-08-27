# 服务端（Linux）

何时用：在 Linux + systemd 上装 hy 服务端、配证书、写 YAML、开防火墙并启动。客户端装法见 [`../client/`](../client/)。

1. 安装脚本、旗标、路径对照、环境变量：[install.md](install.md)
2. 从零配证书并启动：[server.md](server.md)

一步装：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/server/install_server.sh)
```

示例配置：[examples/server.yaml](examples/server.yaml)。
