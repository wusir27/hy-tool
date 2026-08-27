# macOS utun（系统级隧道）

1. 拷贝 [examples/client-macos-utun.yaml](examples/client-macos-utun.yaml)，把 `server` / `auth` / `tls` / `YOUR_SERVER_PUBLIC_IP` 改成真实值：

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
     timeout: 60s
     address:
       ipv4: 100.100.100.101/30
       ipv6: "2001::ffff:ffff:ffff:fff1/126"
     route:
       ipv4Exclude:
         - YOUR_SERVER_PUBLIC_IP/32
   ```

   > 这是 [client.md](client.md) 的附加步骤：SOCKS5 照旧可用，再开一条 utun。完整说明见 [USAGE.md 场景 10](https://github.com/wusir27/hy/blob/main/USAGE.md)。
   >
   > `tun.name` **必须是 `utun` + 数字**，例如 `utun123`。**不能**写成 Linux 那种 `hy0`，否则创建失败。
   >
   > `route.ipv4Exclude` **必须包含服务器公网 IP**（`x.x.x.x/32`），hy **不会**自动填。漏了会把连服务器的包再送进隧道，形成环路。`YOUR_SERVER_PUBLIC_IP` 必须是客户端用来连 `server:` 的那张公网地址，不要填内网 IP。
   >
   > `route:` 三种写法：**不写 `route:`** — 只建网卡、不加路由。**写了 `route:` 但没写 `ipv4`** — 加上一批**拆开的默认路由**（和官方 macOS 客户端一样），不是直接改那一条 default gateway。**显式 `route.ipv4: [0.0.0.0/0]`** — 装**真正的**默认路由；这时务必把服务器 IP 写进 `ipv4Exclude`，否则环路。
   >
   > 上面示例是「写了 `route:`、没写 `ipv4`」。若要真正的默认路由，改成：
   >
   > ```yaml
   >   route:
   >     ipv4: [0.0.0.0/0]
   >     ipv4Exclude:
   >       - YOUR_SERVER_PUBLIC_IP/32
   > ```
   >
   > 配地址、加路由需要 **`sudo`**。创建失败会报错并退出，**不会留下半残网卡**。

2. 用 sudo 启动：

   ```bash
   sudo hy client -c client-macos-utun.yaml
   ```

3. 另开一个终端检查网卡：

   ```bash
   ifconfig utun123
   ```

   > 能看到地址即网卡在。SOCKS5 仍可测：`curl -x socks5h://127.0.0.1:1080 https://www.google.com`。ICMP / ping **不会**走隧道，不要用 `ping` 判断隧道是否工作。

4. 在 `hy client` 那个终端按 Ctrl+C（SIGINT）停止，再确认网卡已消失：

   ```bash
   ifconfig utun123
   ```

   > 停掉后再 `ifconfig utun123` 应不存在。
