# 客户端 0→1

服务端按 [server.md](server.md) 跑起来之后，在本机用同版本 `hy` 做 SOCKS5 / HTTP 代理。完整字段见 [USAGE.md](https://github.com/wusir27/hy/blob/main/USAGE.md)。

本仓库**没有** Windows 安装包；v0.0.1 也未发布 Windows 二进制。

## 下载与 hy version

从 [v0.0.1](https://github.com/wusir27/hy/releases/tag/v0.0.1) 取和本机匹配的文件：

| 平台 | 资源 |
|---|---|
| Linux x86_64 | `hy-linux-amd64` |
| macOS Apple Silicon | `hy-darwin-arm64` |
| macOS Intel | `hy-darwin-amd64` |

```bash
# 按平台改文件名
curl -fL -o hy https://github.com/wusir27/hy/releases/download/v0.0.1/hy-linux-amd64
chmod +x hy
./hy version
```

输出必须是 **`0.0.1`**（与服务端一致）。可把 `hy` 放到 `PATH` 里，下文写成 `hy`。

## 配置

拷贝 [examples/client.yaml](examples/client.yaml)，改三处：

- `server:` → 服务端 **域名或 IP:443**
- `auth:` → 与服务端 `auth.password` 相同（示例是 `secret`）
- `tls.sni` → 与证书 CN/SAN 一致

```yaml
server: YOUR_IP_OR_DOMAIN:443
auth: secret

tls:
  sni: YOUR_IP_OR_DOMAIN
  insecure: true          # 自签：保留。正式证书：删掉这一行
  # ca: /path/to/ca.pem   # 自定义 CA 时用这个，不要 insecure

socks5:
  listen: 127.0.0.1:1080

http:
  listen: 127.0.0.1:8080
```

- **自签**：`tls.sni` + `insecure: true`（跳过校验证书，只适合试验）
- **正式证书 / 自备 CA**：去掉 `insecure`，需要时写 `tls.ca`

`socks5` 是本机 SOCKS5。`http` 可选，浏览器 CONNECT 用。

## 运行

```bash
hy client -c client.yaml
```

不写 `client` / `server` 时按 client 启动。建议始终带 `-c`。退出：Ctrl+C。

## 测试

```bash
curl -x socks5h://127.0.0.1:1080 https://www.google.com
```

`socks5h` 让域名在代理侧解析。浏览器把 SOCKS5 指到 `127.0.0.1:1080`（不要选「代理 DNS」之外的本机直连 DNS，以免分流错乱）。HTTP 代理则是 `127.0.0.1:8080`。

## 不用 hy-client 的 App

官方 Hysteria 2 客户端可以连 hy 服务端。例如 **Shadowrocket** 直接填 Hysteria 2：服务器、UDP 443、密码 `secret`（或你改过的密码），不必再跑 `hy client`。

macOS 若还要系统流量进隧道（utun），见 [macos-utun.md](macos-utun.md)。
