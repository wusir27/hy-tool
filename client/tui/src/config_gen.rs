//! Form state → hy client YAML (v0.0.2 camelCase) and atomic write to ~/.hy/client.yaml.
//!
//! Field names match hy `ClientYaml` (crates/hy-app/src/config.rs). U1 never writes
//! top-level `route.file`. Save does not need the hy binary or the network.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Client-route radios. U1 keeps the choice in memory only; Save must not download
/// or emit `route.file`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RouteMode {
    #[default]
    Off,
    Local,
    Url,
}

/// Config tab fields. Defaults match design §7.2 Darwin skeleton (also parses on Linux).
#[derive(Debug, Clone)]
pub struct FormState {
    pub server: String,
    pub auth: String,
    pub tls_sni: String,
    /// When true, omit `tls.insecure`. When false (default), write `insecure: true`.
    pub verify_cert: bool,
    pub tun_name: String,
    pub tun_ipv4: String,
    pub tun_ipv6: String,
    pub ipv4_exclude: String,
    /// When false, omit `tun.route` entirely (device only).
    pub write_route: bool,
    pub timeout: String,
    pub route_mode: RouteMode,
    pub route_local_path: String,
    pub route_url: String,
    pub advanced_expanded: bool,
    pub bandwidth_up: String,
    pub bandwidth_down: String,
    pub obfs_type: String,
    pub obfs_password: String,
    pub hop_ports: String,
    pub hop_interval: String,
    pub quic_init_stream_window: String,
    pub quic_max_stream_window: String,
    pub quic_init_conn_window: String,
    pub quic_max_conn_window: String,
    pub lazy: bool,
    pub fast_open: bool,
    pub socks5_listen: String,
    /// Advanced: custom hy binary path (U4 Start). Download still targets ~/.hy/bin/hy.
    pub hy_path: String,
}

impl Default for FormState {
    fn default() -> Self {
        Self {
            server: "YOUR_IP_OR_DOMAIN:443".into(),
            auth: "secret".into(),
            tls_sni: "YOUR_IP_OR_DOMAIN".into(),
            verify_cert: false,
            tun_name: "utun123".into(),
            tun_ipv4: "100.100.100.101/30".into(),
            tun_ipv6: String::new(),
            ipv4_exclude: "YOUR_SERVER_PUBLIC_IP/32".into(),
            write_route: true,
            timeout: "60s".into(),
            route_mode: RouteMode::Off,
            route_local_path: String::new(),
            route_url: String::new(),
            advanced_expanded: false,
            bandwidth_up: String::new(),
            bandwidth_down: String::new(),
            obfs_type: String::new(),
            obfs_password: String::new(),
            hop_ports: String::new(),
            hop_interval: String::new(),
            quic_init_stream_window: String::new(),
            quic_max_stream_window: String::new(),
            quic_init_conn_window: String::new(),
            quic_max_conn_window: String::new(),
            lazy: false,
            fast_open: false,
            socks5_listen: String::new(),
            hy_path: String::new(),
        }
    }
}

impl FormState {
    fn server_line(&self) -> String {
        let mut server = self.server.trim().to_string();
        if self.advanced_expanded {
            let hop = self.hop_ports.trim();
            if !hop.is_empty() && !server.contains(',') {
                server = format!("{server},{hop}");
            }
        }
        server
    }
}

/// Render hy-parseable client YAML. Never invents keys; never writes `route.file`.
pub fn to_yaml(form: &FormState) -> String {
    let mut out = String::new();
    push_kv(&mut out, 0, "server", &form.server_line());
    push_kv(&mut out, 0, "auth", form.auth.trim());
    out.push_str("tls:\n");
    push_kv(&mut out, 1, "sni", form.tls_sni.trim());
    if !form.verify_cert {
        out.push_str("  insecure: true\n");
    }

    if form.advanced_expanded {
        emit_bandwidth(&mut out, form);
        emit_obfs(&mut out, form);
        emit_transport(&mut out, form);
        emit_quic(&mut out, form);
        if form.lazy {
            out.push_str("lazy: true\n");
        }
        if form.fast_open {
            out.push_str("fastOpen: true\n");
        }
        let socks = form.socks5_listen.trim();
        if !socks.is_empty() {
            out.push_str("socks5:\n");
            push_kv(&mut out, 1, "listen", socks);
        }
    }

    out.push_str("tun:\n");
    push_kv(&mut out, 1, "name", form.tun_name.trim());
    push_kv(&mut out, 1, "timeout", form.timeout.trim());
    out.push_str("  address:\n");
    push_kv(&mut out, 2, "ipv4", form.tun_ipv4.trim());
    if form.advanced_expanded {
        let v6 = form.tun_ipv6.trim();
        if !v6.is_empty() {
            push_kv(&mut out, 2, "ipv6", v6);
        }
    }
    if form.write_route {
        out.push_str("  route:\n");
        out.push_str("    ipv4Exclude:\n");
        out.push_str("      - ");
        out.push_str(&yaml_scalar(form.ipv4_exclude.trim()));
        out.push('\n');
    }
    out
}

fn emit_bandwidth(out: &mut String, form: &FormState) {
    let up = form.bandwidth_up.trim();
    let down = form.bandwidth_down.trim();
    if up.is_empty() && down.is_empty() {
        return;
    }
    out.push_str("bandwidth:\n");
    if !up.is_empty() {
        push_kv(out, 1, "up", up);
    }
    if !down.is_empty() {
        push_kv(out, 1, "down", down);
    }
}

fn emit_obfs(out: &mut String, form: &FormState) {
    let ty = form.obfs_type.trim();
    let pw = form.obfs_password.trim();
    if ty.is_empty() && pw.is_empty() {
        return;
    }
    let ty = if ty.is_empty() { "salamander" } else { ty };
    out.push_str("obfs:\n");
    push_kv(out, 1, "type", ty);
    if !pw.is_empty() && ty != "plain" {
        out.push_str("  ");
        out.push_str(ty);
        out.push_str(":\n");
        push_kv(out, 2, "password", pw);
    }
}

fn emit_transport(out: &mut String, form: &FormState) {
    let hop = form.hop_interval.trim();
    if hop.is_empty() {
        return;
    }
    out.push_str("transport:\n");
    out.push_str("  udp:\n");
    push_kv(out, 2, "hopInterval", hop);
}

fn emit_quic(out: &mut String, form: &FormState) {
    let fields = [
        (
            "initStreamReceiveWindow",
            form.quic_init_stream_window.trim(),
        ),
        ("maxStreamReceiveWindow", form.quic_max_stream_window.trim()),
        ("initConnReceiveWindow", form.quic_init_conn_window.trim()),
        ("maxConnReceiveWindow", form.quic_max_conn_window.trim()),
    ];
    if fields.iter().all(|(_, v)| v.is_empty()) {
        return;
    }
    out.push_str("quic:\n");
    for (k, v) in fields {
        if !v.is_empty() {
            push_kv(out, 1, k, v);
        }
    }
}

fn push_kv(out: &mut String, indent: usize, key: &str, value: &str) {
    for _ in 0..indent {
        out.push_str("  ");
    }
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&yaml_scalar(value));
    out.push('\n');
}

fn yaml_scalar(s: &str) -> String {
    if yaml_plain_ok(s) {
        s.to_string()
    } else {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        format!("\"{escaped}\"")
    }
}

fn yaml_plain_ok(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.starts_with([' ', '\t']) || s.ends_with([' ', '\t']) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~" | "y" | "n"
    ) {
        return false;
    }
    if s.contains('\n') || s.contains('\r') || s.contains('#') || s.contains(": ") {
        return false;
    }
    if s.starts_with([
        '&', '*', '!', '%', '@', '`', '\'', '"', '{', '}', '[', ']', ',', '?', '|', '>', '-',
    ]) {
        return false;
    }
    true
}

pub fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))
}

/// Create `~/.hy` (0700 if missing) and atomically write `client.yaml` (0600).
pub async fn save_to_home(form: &FormState) -> Result<PathBuf> {
    save_to(form, &home_dir()?).await
}

pub async fn save_to(form: &FormState, home: &Path) -> Result<PathBuf> {
    let yaml = to_yaml(form);
    let hy_dir = home.join(".hy");
    ensure_hy_dir(&hy_dir).await?;

    let dest = hy_dir.join("client.yaml");
    let tmp = hy_dir.join(format!(".client.yaml.{}.tmp", std::process::id()));
    tokio::fs::write(&tmp, yaml.as_bytes())
        .await
        .with_context(|| format!("write {}", tmp.display()))?;
    set_mode(&tmp, 0o600)?;
    tokio::fs::rename(&tmp, &dest)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    set_mode(&dest, 0o600)?;
    Ok(dest)
}

async fn ensure_hy_dir(hy_dir: &Path) -> Result<()> {
    match tokio::fs::metadata(hy_dir).await {
        Ok(meta) => {
            if !meta.is_dir() {
                anyhow::bail!("{} exists and is not a directory", hy_dir.display());
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            tokio::fs::create_dir_all(hy_dir)
                .await
                .with_context(|| format!("create {}", hy_dir.display()))?;
            set_mode(hy_dir, 0o700)?;
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("stat {}", hy_dir.display())),
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("metadata {}", path.display()))?
        .permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Subset of hy `ClientYaml` used to assert parse shape (camelCase, auth is a string).
    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ClientShape {
        server: Option<String>,
        auth: Option<String>,
        tls: Option<TlsShape>,
        tun: Option<TunShape>,
        route: Option<serde_yaml::Value>,
        #[allow(dead_code)]
        socks5: Option<serde_yaml::Value>,
        #[allow(dead_code)]
        bandwidth: Option<serde_yaml::Value>,
        #[allow(dead_code)]
        obfs: Option<serde_yaml::Value>,
        #[allow(dead_code)]
        quic: Option<serde_yaml::Value>,
        #[allow(dead_code)]
        lazy: Option<bool>,
        #[serde(rename = "fastOpen")]
        #[allow(dead_code)]
        fast_open: Option<bool>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct TlsShape {
        sni: Option<String>,
        insecure: Option<bool>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TunShape {
        name: Option<String>,
        timeout: Option<String>,
        address: Option<AddrShape>,
        route: Option<TunRouteShape>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct AddrShape {
        ipv4: Option<String>,
        #[allow(dead_code)]
        ipv6: Option<String>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TunRouteShape {
        ipv4_exclude: Option<Vec<String>>,
    }

    fn parse(yaml: &str) -> ClientShape {
        serde_yaml::from_str(yaml)
            .unwrap_or_else(|e| panic!("serde_yaml parse failed: {e}\n{yaml}"))
    }

    fn value(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("value parse failed: {e}\n{yaml}"))
    }

    #[test]
    fn default_yaml_contains_required_keys() {
        let yaml = to_yaml(&FormState::default());
        assert!(yaml.contains("server:"), "{yaml}");
        assert!(yaml.contains("auth:"), "{yaml}");
        assert!(yaml.contains("sni:"), "{yaml}");
        assert!(
            yaml.contains("insecure: true"),
            "default 校验证书 off must write insecure: true\n{yaml}"
        );
        assert!(yaml.contains("name:"), "{yaml}");
        assert!(
            yaml.contains("timeout: 60s"),
            "timeout must be unquoted 60s\n{yaml}"
        );
        assert!(yaml.contains("ipv4:"), "{yaml}");
        assert!(
            yaml.contains("ipv4Exclude"),
            "must emit camelCase ipv4Exclude\n{yaml}"
        );
        assert!(
            !yaml.contains("route.file") && !yaml.contains("file:"),
            "U1 must not write route.file\n{yaml}"
        );

        let v = value(&yaml);
        assert!(v.get("route").is_none(), "no top-level route:\n{yaml}");
        assert_eq!(
            v.get("tls")
                .and_then(|t| t.get("insecure"))
                .and_then(|x| x.as_bool()),
            Some(true)
        );
        let tun = v.get("tun").expect("tun");
        assert_eq!(tun.get("name").and_then(|x| x.as_str()), Some("utun123"));
        assert_eq!(tun.get("timeout").and_then(|x| x.as_str()), Some("60s"));
        assert_eq!(
            tun.get("address")
                .and_then(|a| a.get("ipv4"))
                .and_then(|x| x.as_str()),
            Some("100.100.100.101/30")
        );
        let exclude = tun
            .get("route")
            .and_then(|r| r.get("ipv4Exclude"))
            .and_then(|x| x.as_sequence())
            .expect("ipv4Exclude list");
        assert_eq!(exclude[0].as_str(), Some("YOUR_SERVER_PUBLIC_IP/32"));
        assert!(tun.get("address").and_then(|a| a.get("ipv6")).is_none());
        assert!(v.get("socks5").is_none());
    }

    #[test]
    fn write_route_off_omits_tun_route() {
        let mut form = FormState::default();
        form.write_route = false;
        let yaml = to_yaml(&form);
        let v = value(&yaml);
        let tun = v.get("tun").expect("tun");
        assert!(
            tun.get("route").is_none(),
            "write route off must omit tun.route\n{yaml}"
        );
        assert!(
            !yaml.contains("route:"),
            "no route: key when write_route is off\n{yaml}"
        );
        assert!(yaml.contains("name:"), "{yaml}");
        assert!(yaml.contains("ipv4:"), "{yaml}");
    }

    #[test]
    fn verify_cert_omits_insecure() {
        let mut form = FormState::default();
        form.verify_cert = true;
        let yaml = to_yaml(&form);
        let v = value(&yaml);
        let tls = v.get("tls").expect("tls");
        assert!(
            tls.get("insecure").is_none(),
            "校验证书 on must omit tls.insecure\n{yaml}"
        );
        assert!(
            !yaml.contains("insecure"),
            "must not write insecure: false\n{yaml}"
        );
        assert!(yaml.contains("sni:"), "{yaml}");
    }

    #[test]
    fn serde_required_keys_and_auth_is_string() {
        let yaml = to_yaml(&FormState::default());
        let shape = parse(&yaml);
        assert_eq!(shape.server.as_deref(), Some("YOUR_IP_OR_DOMAIN:443"));
        assert_eq!(shape.auth.as_deref(), Some("secret"));
        let tls = shape.tls.expect("tls");
        assert_eq!(tls.sni.as_deref(), Some("YOUR_IP_OR_DOMAIN"));
        assert_eq!(tls.insecure, Some(true));
        let tun = shape.tun.expect("tun");
        assert_eq!(tun.name.as_deref(), Some("utun123"));
        assert_eq!(tun.timeout.as_deref(), Some("60s"));
        assert_eq!(
            tun.address.as_ref().and_then(|a| a.ipv4.as_deref()),
            Some("100.100.100.101/30")
        );
        assert_eq!(
            tun.route
                .as_ref()
                .and_then(|r| r.ipv4_exclude.clone())
                .unwrap(),
            vec!["YOUR_SERVER_PUBLIC_IP/32".to_string()]
        );
        assert!(shape.route.is_none());

        let v = value(&yaml);
        assert!(
            v.get("auth").and_then(|a| a.as_str()).is_some(),
            "auth must be a YAML string, not a map\n{yaml}"
        );
    }

    #[test]
    fn route_mode_url_still_omits_route_file() {
        let mut form = FormState::default();
        form.route_mode = RouteMode::Url;
        form.route_url = "https://example.com/sr.conf".into();
        form.route_local_path = "/tmp/route.conf".into();
        let yaml = to_yaml(&form);
        let v = value(&yaml);
        assert!(v.get("route").is_none(), "{yaml}");
        assert!(!yaml.contains("route.file"), "{yaml}");
        assert!(
            !yaml.contains("/tmp/route.conf"),
            "must not persist local path into yaml in U1\n{yaml}"
        );
        assert!(
            !yaml.contains("https://example.com/sr.conf"),
            "must not persist URL into yaml in U1\n{yaml}"
        );
        form.hy_path = "/opt/custom/hy".into();
        let yaml = to_yaml(&form);
        assert!(
            !yaml.contains("/opt/custom/hy"),
            "hy path is TUI-only and must not be written to client.yaml\n{yaml}"
        );
    }

    #[test]
    fn empty_advanced_omits_optional_keys() {
        let yaml = to_yaml(&FormState::default());
        let v = value(&yaml);
        assert!(v.get("bandwidth").is_none());
        assert!(v.get("obfs").is_none());
        assert!(v.get("quic").is_none());
        assert!(v.get("transport").is_none());
        assert!(v.get("socks5").is_none());
        assert!(v.get("lazy").is_none());
        assert!(v.get("fastOpen").is_none());
    }

    #[test]
    fn expanded_advanced_emits_filled_camel_case() {
        let mut form = FormState::default();
        form.advanced_expanded = true;
        form.bandwidth_up = "100mbps".into();
        form.bandwidth_down = "500mbps".into();
        form.obfs_type = "salamander".into();
        form.obfs_password = "abcd".into();
        form.hop_ports = "10000-20000".into();
        form.hop_interval = "30s".into();
        form.quic_init_stream_window = "8388608".into();
        form.lazy = true;
        form.fast_open = true;
        form.socks5_listen = "127.0.0.1:1080".into();
        form.tun_ipv6 = "2001::ffff:ffff:ffff:fff1/126".into();
        let yaml = to_yaml(&form);
        let v = value(&yaml);
        assert_eq!(
            v.get("server").and_then(|x| x.as_str()),
            Some("YOUR_IP_OR_DOMAIN:443,10000-20000")
        );
        assert!(v.get("bandwidth").is_some(), "{yaml}");
        assert!(v.get("obfs").is_some(), "{yaml}");
        assert_eq!(
            v.get("transport")
                .and_then(|t| t.get("udp"))
                .and_then(|u| u.get("hopInterval"))
                .and_then(|x| x.as_str()),
            Some("30s")
        );
        assert!(yaml.contains("initStreamReceiveWindow"), "{yaml}");
        assert_eq!(v.get("lazy").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(v.get("fastOpen").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(
            v.get("socks5")
                .and_then(|s| s.get("listen"))
                .and_then(|x| x.as_str()),
            Some("127.0.0.1:1080")
        );
        assert_eq!(
            v.get("tun")
                .and_then(|t| t.get("address"))
                .and_then(|a| a.get("ipv6"))
                .and_then(|x| x.as_str()),
            Some("2001::ffff:ffff:ffff:fff1/126")
        );
        parse(&yaml);
    }

    #[tokio::test]
    async fn write_temp_home_mode_0600() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("hy-tui-home-{nanos}-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &tmp);

        let path = save_to_home(&FormState::default())
            .await
            .expect("save_to_home");

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }

        assert_eq!(path, tmp.join(".hy").join("client.yaml"));
        assert!(path.is_absolute(), "{path:?}");
        let yaml = std::fs::read_to_string(&path).unwrap();
        parse(&yaml);
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "client.yaml must be 0600");
        let dir_mode = std::fs::metadata(tmp.join(".hy"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, ".hy must be 0700 when created");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Parse-compat against hy v0.0.2: generated YAML must not fail config parse.
    /// Placeholders in the default skeleton (`YOUR_SERVER_PUBLIC_IP/32`) fail hy
    /// fill as a bad CIDR — same as USAGE examples — so this uses valid dummy
    /// values. Connect failure / `timeout` exit 124 is OK.
    /// If the binary cannot be fetched, the serde shape tests above still cover
    /// camelCase keys (hy `ClientYaml` in crates/hy-app/src/config.rs).
    #[test]
    fn hy_v002_binary_parses_generated_yaml() {
        let mut form = FormState::default();
        form.server = "127.0.0.1:443".into();
        form.tls_sni = "localhost".into();
        form.ipv4_exclude = "203.0.113.1/32".into();
        let yaml = to_yaml(&form);
        parse(&yaml);

        let bin = match fetch_hy_linux_amd64() {
            Some(p) => p,
            None => {
                eprintln!(
                    "skip hy binary parse: could not download v0.0.2 hy-linux-amd64 (serde shape test still ran)"
                );
                return;
            }
        };
        let dir = std::env::temp_dir().join(format!("hy-tui-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("client.yaml");
        std::fs::write(&cfg, &yaml).unwrap();

        let output = std::process::Command::new("timeout")
            .args(["2", bin.to_str().unwrap(), "client", "-c"])
            .arg(&cfg)
            .output()
            .expect("run hy client");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");
        let code = output.status.code();
        // timeout(1) uses 124 when the process was still running → YAML filled.
        let timed_out = code == Some(124);
        assert!(
            timed_out || !is_yaml_parse_failure(&combined),
            "hy rejected generated YAML (exit {code:?}):\n{combined}\n--- yaml ---\n{yaml}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn is_yaml_parse_failure(msg: &str) -> bool {
        let lower = msg.to_ascii_lowercase();
        lower.contains("invalid config")
            || lower.contains("unknown field")
            || lower.contains("while parsing")
            || lower.contains("did not find expected")
            || (lower.contains("yaml") && !lower.contains("client.yaml"))
    }

    fn fetch_hy_linux_amd64() -> Option<PathBuf> {
        let dest = PathBuf::from("/tmp/hy-linux-amd64");
        if dest.is_file() {
            return Some(dest);
        }
        let url = "https://github.com/wusir27/hy/releases/download/v0.0.2/hy-linux-amd64";
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "-o"])
            .arg(&dest)
            .arg(url)
            .status()
            .ok()?;
        if !status.success() || !dest.is_file() {
            let _ = std::fs::remove_file(&dest);
            return None;
        }
        let _ = std::process::Command::new("chmod")
            .args(["+x"])
            .arg(&dest)
            .status();
        Some(dest)
    }
}
