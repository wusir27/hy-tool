# hy Linux server install

Install, upgrade, or remove the [hy](https://github.com/wusir27/hy) server on Linux + systemd.

There is **no** short domain. This script does **not** occupy or imitate `get.hy2.sh`.

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/install/install_server.sh)
```

If GitHub has no published Release yet, download will fail. Use a local gnu binary instead:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/wusir27/hy-tool/main/install/install_server.sh) --local /path/to/hy-linux-amd64
```

## Flags

| Flag | Action |
|---|---|
| *(none)* | Install or upgrade to the latest GitHub Release |
| `--version VER` | Install tag `v0.3.10` or `0.3.10` (mutually exclusive with `--local`) |
| `-l` / `--local FILE` | Install a local binary (no download) |
| `-f` / `--force` | Reinstall even if already latest |
| `-c` / `--check` | Compare installed version to latest Release |
| `--remove` | Remove `/usr/local/bin/hy` and hy systemd units only |
| `-h` / `--help` | Usage |

New install does **not** `systemctl enable` or `start`. Edit the example config, then:

```bash
nano /etc/hy/server.yaml
systemctl enable --now hy-server.service
```

USAGE: https://github.com/wusir27/hy/blob/main/USAGE.md

## Paths vs official Hysteria

Same machine may run both. This script never writes official paths.

| | Official | hy |
|---|---|---|
| Binary | `/usr/local/bin/hysteria` | `/usr/local/bin/hy` |
| Config dir | `/etc/hysteria` | `/etc/hy` |
| Default config **file** | `config.yaml` | **`server.yaml`** |
| Default config path | `/etc/hysteria/config.yaml` | `/etc/hy/server.yaml` |
| Units | `hysteria-server.service` / `@` | `hy-server.service` / `hy-server@.service` |
| User / home | `hysteria` / `/var/lib/hysteria` | `hy` / `/var/lib/hy` |
| Script user env | `HYSTERIA_USER` | `HY_USER` (does not read `HYSTERIA_USER`) |

`--remove` keeps `/etc/hy` and user `hy`. It prints `rm` / `userdel` hints. Official files are left alone.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `HY_USER` | `hy` | systemd `User=` / `Group=` |
| `HY_HOME_DIR` | `/var/lib/$HY_USER` | home / ACME dir base |
| `ARCHITECTURE` | from `uname -m` | `amd64` / `arm64` / `armv7` / `386` |
| `HY_LIBC` | `gnu` | `musl` only if that Release actually has `hy-linux-$ARCH-musl` |
| `FORCE_NO_ROOT=1` | off | do not `sudo` |
| `FORCE_NO_SYSTEMD=1` | off | continue if systemd is missing |
| `FORCE_NO_SYSTEMD=2` | off | skip all systemd commands |
| `FORCE_NO_SELINUX=1` | off | skip `chcon` |
| `ALL_PROXY` | empty | passed through to curl |

`ARCHITECTURE=amd64-avx` is rejected: hy has no avx artifact.

If you set `HY_USER=hysteria`, the script warns that home/ACME may overlap with official.
