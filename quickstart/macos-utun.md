# macOS utun（系统级隧道）

这是 [client.md](client.md) 的附加步骤：SOCKS5 照旧可用，再开一条 utun，让系统路由进隧道。完整说明见 [USAGE.md 场景 10](https://github.com/wusir27/hy/blob/main/USAGE.md)。

## 约束

- `tun.name` **必须是 `utun` + 数字**，例如 `utun123`。**不能**写成 Linux 那种 `hy0`，否则创建失败。
- 配地址、加路由需要 **`sudo`**。
- `route.ipv4Exclude` **必须包含服务器公网 IP**（`x.x.x.x/32`），hy **不会**自动填。漏了会把连服务器的包再送进隧道，形成环路。
- ICMP / ping **不会**走隧道。
- 创建失败会报错并退出，**不会留下半残网卡**。

## `route:` 三种写法（摘自 USAGE）

- **不写 `route:`**：只建网卡、不加路由。
- **写了 `route:` 但没写 `ipv4`**：加上一批**拆开的默认路由**（和官方 macOS 客户端一样），不是直接改那一条 default gateway。
- **显式 `route.ipv4: [0.0.0.0/0]`**：装**真正的**默认路由。这时务必把服务器 IP 写进 `ipv4Exclude`，否则环路。

## 配置

拷贝 [examples/client-macos-utun.yaml](examples/client-macos-utun.yaml)，把 `server` / `auth` / `tls` / `YOUR_SERVER_PUBLIC_IP` 改成真实值。示例把 SOCKS5 和 utun 写在同一份配置里：

```yaml
server: YOUR_IP_OR_DOMAIN:443
auth: secret
tls:
  sni: YOUR_IP_OR_DOMAIN
  insecure: true

socks5:
  listen: 127.0.0.1:1080

tun:
  name: utun123
  mtu: 1500
  timeout: 5m
  address:
    ipv4: 100.100.100.101/30
    ipv6: "2001::ffff:ffff:ffff:fff1/126"
  route:
    ipv4Exclude:
      - YOUR_SERVER_PUBLIC_IP/32
```

上面是「写了 `route:`、没写 `ipv4`」：拆开的默认路由。若要真正的默认路由，改成：

```yaml
  route:
    ipv4: [0.0.0.0/0]
    ipv4Exclude:
      - YOUR_SERVER_PUBLIC_IP/32
```

`YOUR_SERVER_PUBLIC_IP` 必须是客户端用来连 `server:` 的那张公网地址，不要填内网 IP。

## 启动 / 停止 / 检查

```bash
sudo hy client -c client-macos-utun.yaml
```

另开一个终端：

```bash
ifconfig utun123
```

能看到地址即网卡在。停：在 `hy client` 那个终端 Ctrl+C（SIGINT）。停掉后再 `ifconfig utun123` 应不存在。

SOCKS5 仍可测：

```bash
curl -x socks5h://127.0.0.1:1080 https://www.google.com
```

不要用 `ping` 判断隧道是否工作。
