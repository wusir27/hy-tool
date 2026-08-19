# Linux 服务端 0→1

目标：在一台 Linux 机器上跑 `hy` v0.0.1，监听 UDP 443，配置与 [examples/server.yaml](examples/server.yaml) 一致。

## 1. 前提

- Linux + systemd，用 **root** 跑安装脚本
- 云厂商安全组 / 本机防火墙放行 **UDP 443**（QUIC；不是 TCP 443）
- 一个域名或公网 IP，给客户端填 `server:`
- 自签证书需要 `openssl`

本机可以同时装着官方 Hysteria。hy 用自己的路径，**不会**写 `/etc/hysteria`。

## 2. 安装

v0.0.1 已发布，默认下载可用：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/install/install_server.sh)
```

钉死版本（可选）：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/install/install_server.sh) --version v0.0.1
```

脚本会装这些（不碰官方路径）：

| 项 | 路径 |
|---|---|
| 二进制 | `/usr/local/bin/hy` |
| 配置 | `/etc/hy/server.yaml` |
| 单元 | `hy-server.service`（另有 `hy-server@.service`） |
| 运行用户 / 家目录 | `hy` / `/var/lib/hy` |

详情与环境变量见 [install/README.md](../install/README.md)。

## 3. 新装不会启动服务

脚本**不会** `enable` / `start`。先改配置和证书，再手动开。

安装脚本自带的示例是 ACME 占位配置。本指南改用手写证书（`tls.cert` / `tls.key`），把 `/etc/hy/server.yaml` 整份换成下面的例子。

## 4. 证书

证书必须放进 `/etc/hy/`，并且 **`hy` 用户能读**。YAML 里请写绝对路径：

```yaml
tls:
  cert: /etc/hy/server.crt
  key: /etc/hy/server.key
```

用户原稿里的 `server.crt` / `server.key` 是**相对路径**，相对的是进程工作目录。systemd 单元的 `WorkingDirectory` 是 `~`，即 `/var/lib/hy`，**不是** `/etc/hy/`。相对路径会去找 `/var/lib/hy/server.crt`。建议一律写成 `/etc/hy/...`。

**自签（试验）**，CN 填客户端将使用的 SNI（域名或你约定的名字）：

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout /etc/hy/server.key -out /etc/hy/server.crt -days 365 -nodes \
  -subj "/CN=YOUR_DOMAIN_OR_IP"
```

**已有正式证书**：把完整链和私钥拷进 `/etc/hy/server.crt`、`/etc/hy/server.key`（例如 Let's Encrypt 的 `fullchain.pem` / `privkey.pem`）。本指南不用 ACME；hy 的 ACME 只支持 HTTP-01 / TLS-ALPN-01，**不支持 DNS-01**。

## 5. 写入配置

```bash
cp /path/to/hy-tool/quickstart/examples/server.yaml /etc/hy/server.yaml
# 或直接编辑 /etc/hy/server.yaml，内容见该文件
```

把 `auth.password` 从 `secret` 改成你自己的密码（客户端 `auth` 必须相同）。

各块含义：

### `listen`（注释掉）

`# listen: :443` 等于用默认值 **`:443`**：所有网卡、UDP 443。

### `tls`

手写 PEM 证书。和 `acme` 不能同时写。

### `auth`

`type: password`，整串对上即通过；用户名记成 `user`。客户端写成同一个字符串。

### `masquerade`

未通过 auth 的访问（失败的 `/auth`）伪装成反向代理到 `https://news.163.com/`。`rewriteHost: true` 会改 Host。正常登录用户不受影响。本例没有 `listenHTTP` / `listenHTTPS`，**不会**额外开 TCP 80/443。

### `#bandwidth` + `ignoreClientBandwidth: true`

带宽块注释掉，服务端不声明上下行。`ignoreClientBandwidth: true` 时不管客户端报了多少带宽。双方都写了带宽、且服务端**没有** `ignoreClientBandwidth` 时才用 Brutal；否则用 **BBR**（默认拥塞控制）。此配置走 BBR。

### `disableUDP` / `udpIdleTimeout`

`disableUDP: false`：TCP 和 UDP 都代理。`udpIdleTimeout: 60s`：UDP session 闲置回收，与默认相同。

### `acl.inline`

上到下先匹配先生效：

1. `reject(geoip:cn)` — 解析后的中国 IP 拒绝
2. 三条 RFC1918：`10/8`、`172.16/12`、`192.168/16` 拒绝

没命中的走名为 `default` 的 outbound；没有 `default` 就走 **列表第一项** `v4_only`（仅 IPv4）。`v6_only` 只有 ACL 动作写成这个名字才会用到，本例不会命中。

`geoip:` 用 V2Ray `.dat`。没写 `acl.geoip` 时，在**运行目录**找 `geoip.dat`，没有就自动下载（默认最多 7 天下一次）。systemd 下运行目录是 `/var/lib/hy`。**第一次启动需要能访问外网**（下载 [geoip.dat](https://cdn.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geoip.dat)）。库里没有对应国家码会**启动失败**，不会假装没这条规则。`geoip` 只匹配已经是 IP 的目标；域名要先经下面的 resolver 解析。

### `resolver`

`type: tls` = DoT，实际用的是 `resolver.tls`：**`1.1.1.1:853`**，SNI `cloudflare-dns.com`。上面的 `udp:` 块只有 `type: udp` 才会用到，本例**闲置**。配了 ACL 必须有 DNS。

### `outbounds`

- `v4_only`：`direct.mode: 4`，只用 IPv4，没有 A 记录就失败
- `v6_only`：`mode: 6`，只用 IPv6（本例 ACL 未引用）

## 6. 权限

```bash
chown hy:hy /etc/hy/server.yaml /etc/hy/server.crt /etc/hy/server.key
chmod 640 /etc/hy/server.key
chmod 640 /etc/hy/server.yaml
```

私钥 `640`、属主 `hy`。配置里有密码，同样不要世界可读。`hy` 读不到证书时服务会起不来。

## 7. 启动与核对

```bash
systemctl enable --now hy-server
journalctl -u hy-server -e
hy version
```

`hy version` 应为 `0.0.1`。日志里若在拉 `geoip.dat`，等下载完成；失败会直接退出。

## 8. 防火墙

只需要 **UDP 443**，不必为 hy 再开别的端口。

```bash
# ufw
ufw allow 443/udp

# firewalld
firewall-cmd --permanent --add-port=443/udp && firewall-cmd --reload
```

云安全组同样放行 UDP 443。TCP 443 不是本配置所需。

## 9. 下一步

到 [client.md](client.md) 连这台机器。客户端 `auth` 填同一密码，`server` 填 `域名或IP:443`。
