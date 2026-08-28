# 终端界面启动器（hy-tui）

`hy-tui` 是 [hy-tool](https://github.com/wusir27/hy-tool) 里的启动器，**不是**第二套客户端。它下载官方 `hy`、写 `~/.hy/client.yaml`（可选再写 `~/.hy/route.conf`），然后 exec `hy client`。完整字段见 [USAGE.md](https://github.com/wusir27/hy/blob/main/USAGE.md)。

> SOCKS5 手写步骤见 [client.md](client.md)。macOS utun 背景（网卡名、exclude、sudo）见 [macos-utun.md](macos-utun.md)。本页只讲 TUI。

两个 tab：`1 Config` / `2 Run`。Start 成功后切到 Run。`1` / `2` 或 Tab 切换。Esc / Ctrl+C 退出。

---

## 1. 安装 / 打开

面向 macOS / Linux。**没有 Windows TUN**（hy 本身也不支持）。

1. 安装脚本（推荐，不需要 Rust）：

   ```bash
   bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/client/install_tui.sh)
   ```

   默认写到 `~/.hy/bin/hy-tui`。可用 `HY_TUI_TAG` 指定 Release 标签，`HY_TUI_DIR` 改安装目录。

2. 或从 [Releases](https://github.com/wusir27/hy-tool/releases) 手动下载匹配本机的资产（`hy-tui-darwin-arm64` / `hy-tui-darwin-amd64` / `hy-tui-linux-amd64` / `hy-tui-linux-arm64`），放到 `~/.hy/bin/hy-tui` 并 `chmod 755`。

3. 运行：

   ```bash
   ~/.hy/bin/hy-tui
   ```

   或把 `~/.hy/bin` 加进 PATH 后直接 `hy-tui`。TUN 仍需要 sudo（见第 6 节）。服务端须已按 [server.md](../server/server.md) 跑起来，版本与客户端 `hy` 一致。

### 从源码编

需要 **Rust 1.85+**。

```bash
git clone https://github.com/wusir27/hy-tool.git
cd hy-tool/client/tui
cargo run --release
```

或从仓库根目录：`cargo install --path client/tui`（二进制通常在 `~/.cargo/bin/hy-tui`）。

---

## 2. 第一次运行（下载 hy）

打开后若 `~/.hy/bin/hy` 不存在，TUI 会按本机 OS/CPU 从 [wusir27/hy](https://github.com/wusir27/hy) 的 GitHub Release 拉匹配资产（如 `hy-linux-amd64`、`hy-darwin-arm64`），并用 `SHA256SUMS` 校验，通过后 chmod 755 写到 `~/.hy/bin/hy`。

> 无网、校验失败、没有对应资产：Config 顶栏红字，不覆盖已有好文件。以后用底栏 **Update hy** 再下一次（仍校验 SHA256，始终写入 `~/.hy/bin/hy`）。高级里 **hy path** 可指向已有二进制；留空则用 `~/.hy/bin/hy`。

---

## 3. Config：填参数、Save

Config 分组：**连接** / **TUN** / **路由** / **高级**（默认折叠）。↑↓ 移动，文本框直接改，勾选/单选用 Space 或 Enter。底栏：**Save** / **Start** / **Update hy**。

1. **连接**：`server`（`IP或域名:443`）、`auth`、`tls.sni`、**校验证书**（关 → `tls.insecure: true`，试验用自签；正式证书请打开校验）。

2. **TUN**：

   | 字段 | 说明 |
   |---|---|
   | `name` | Darwin **必须** `utun`+数字（默认 `utun123`）。Linux 改成如 `hy0`，不要用 `utun`。 |
   | `address.ipv4` | 默认 `100.100.100.101/30` |
   | `ipv4Exclude` | **必须**填服务器公网 IP/`32`（见下一节） |
   | `write route:` | 默认开，往 yaml 写 `tun.route:`（USAGE 场景 10）。关 → 只建网卡、不加路由 |
   | `timeout` | 示例 `60s` |

3. **路由**：`(•) off` / `local` / `url`。off：Start **不加** `--route`。local：已有 `.conf`，文件必须存在。url：见第 5 节。

4. **Save**：只写 `~/.hy/client.yaml`，**不**启动 hy。Save **不**写 `route.file`。空的高级字段不会进 yaml。

下次打开会把 `~/.hy/client.yaml` 读回表单，规则缓存（off/local/url、URL、本地路径）从 `~/.hy/tui.json` 读回。

> 高级（展开后）：`bandwidth`、`obfs`、hop 端口、`quic` 窗口、`address.ipv6`、`lazy` / `fastOpen`、可选 `socks5.listen`、`hy path`。不需要就保持折叠。

---

## 4. ipv4Exclude（开路由前必改）

打开 **write route:**（要装默认路由 / 拆开的默认路由）之前，把 `ipv4Exclude` 改成**真实服务器公网 IP/32**，也就是本机连 `server:` 用的那张公网地址。

> 默认占位 `YOUR_SERVER_PUBLIC_IP/32` **不能**拿去装默认路由。hy **不会**自动填 exclude。漏了或填内网 IP，会把连服务器的包再送进隧道，形成环路。TUI 不替你解析公网 IP。

---

## 5. 规则 URL

路由选 **url**，贴 **HTTPS** 地址（`http://` 拒绝）。Start 时下载到 `~/.hy/route.conf`（上限 8 MiB），再给 hy 加 `--route <绝对路径>`。off 则省略 `--route`。local 则 `--route` 指向你填的绝对路径，不经过这次下载。

> TUI 把规则当数据文件，不执行、不解析语义。hy 启动时加载；**没有热加载**，改 URL 或换文件后必须 **Restart**。
>
> 正文几乎全是 `RULE-SET`（如 lazy.conf）时，顶栏警告 **「这份规则 hy 用不了」**（hy 会跳过 RULE-SET），仍允许启动。不要指望 TUI 去拉 RULE-SET 或做策略组。

---

## 6. Start（sudo）

Config 底栏 **Start**（或 Run 里 **Restart**）。TUI **保持普通用户**，只对 `~/.hy/bin/hy` 走系统 `sudo` 拉起 `hy client -c ~/.hy/client.yaml`（有规则再加 `--route`）。工作目录是 `~/.hy`。

1. 第一次（没有 sudo timestamp）：弹出 **sudo** 框，提示「系统密码，给 sudo 用一次」。Enter 提交，Esc 取消。密码只进 sudo 的 stdin 一次，**不写磁盘**、不进日志。
2. 已是 root，或 `sudo -n` 已缓存 timestamp：不再问密码。短时间 Restart 通常也不再问。
3. 取消或连错三次：停在 Config，不启动 hy。没有 TTY：提示「在 Terminal 里跑 hy-tui」。

> macOS utun / Linux TUN 配地址和加路由需要提权，这是 OS 的要求。TUI 不绕过、不做 setuid、不以 root 常驻。

---

## 7. Run：状态、日志、速率

Run 顶栏：`status` / `server` / `tun` / `pid` / `uptime`。下面是 **TUN 出**、**TUN 入**（速率、累计、近 60s sparkline）和 **log**（`hy client` 的 stdout+stderr，环形约 2000 行）。底栏：**Stop** / **Restart** / **Clear log**。

状态只有四种：

| 状态 | 含义 |
|---|---|
| STOPPED | 未运行 |
| STARTING | 正在拉起；或正在停 |
| CONNECTED | 子进程还在，**并且**日志里出现成功行（如 `tun up` / `authenticated`） |
| ERROR | 进程退出；日志原样保留 |

> CONNECTED 不解析协议，只看进程 + 现有成功日志。Clear log 只清环形缓冲，不停 hy。

### 速率是 TUN ifstats，不是 QUIC

界面标 **TUN 出/入**。这是虚拟网卡上的 IP 包，**不是** QUIC/UDP 线字节。

| 平台 | 读什么 |
|---|---|
| Linux | sysfs `rx_bytes` / `tx_bytes`（`/sys/class/net/<name>/statistics/`） |
| macOS | `getifaddrs` 的 `ifi_ibytes` / `ifi_obytes`（与 `netstat -ibn -I <name>` 同一套） |

极性：出 = Linux `rx_bytes` / Darwin `ifi_ibytes`（内核交给 TUN、hy 从 fd 读走）；入 = Linux `tx_bytes` / Darwin `ifi_obytes`（hy 写入 TUN、内核交给本机）。约 1 秒采样。

> 开了 `--route` 时，进 TUN 的包包括后来被 DIRECT / REJECT 的，计数 ≥ 真正上隧道的用户流量。TUI 不假装这是纯隧道。无 `--route` 时，TUN 上的用户 IP 流量近似进隧道的用户流量（仍不含 QUIC/TLS 开销）。
>
> 网卡还不存在、读不到、或 **Stop 之后**：显示破折号 `—`，不发明 0。

---

## 8. Stop

Run 底栏 **Stop** = 对 hy 的 pid 发 **SIGINT**（与 USAGE 的 Ctrl+C 相同），超时再 **SIGTERM**。常规路径不用 SIGKILL。需要提权结束 root 的 hy 时走 `sudo -n kill`（timestamp 通常还在）。

> Stop 之后 tun / utun 应消失（Linux：`ip link`；macOS：`ifconfig <name>`）。创建失败时 hy 会报错退出，不留半残网卡。ICMP / ping **不会**走隧道，不要用 ping 判断是否通。

---

## 9. 本机数据

一律用户家目录（绝对路径进 yaml 和命令行）：

```
~/.hy/
  bin/hy          # 下载的官方 hy
  client.yaml     # TUI Save / Start 写出
  tui.json        # 0600；规则单选 off/local/url、URL、本地路径；不含 auth
  route.conf      # 可选；url 模式下载到这里
```

`auth` 只进 `client.yaml`。TUI 不做单独密钥库。

---

## 10. 相关文档

- SOCKS5 0→1：[client.md](client.md)
- macOS utun 手写 yaml：[macos-utun.md](macos-utun.md)
- hy 字段与场景：[USAGE.md](https://github.com/wusir27/hy/blob/main/USAGE.md)
