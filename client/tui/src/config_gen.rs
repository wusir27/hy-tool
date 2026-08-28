//! Form state ↔ hy client YAML (v0.0.2 camelCase) and atomic write to ~/.hy/client.yaml.
//!
//! Field names match hy `ClientYaml` (crates/hy-app/src/config.rs). U1 never writes
//! top-level `route.file`. Save does not need the hy binary or the network.
//! Startup may read `~/.hy/client.yaml` into the form; a missing file stays default.
//! Unreadable or invalid yaml returns an error and must not rewrite the file.
//!
//! Route radios (off/local/url + URL + local path) are persisted separately in
//! `~/.hy/tui.json` (0600). That file never contains auth, sudo password, or hy path.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Client-route radios. Persist to `tui.json` on Save/Start; Save must not download
/// or emit `route.file`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RouteMode {
    #[default]
    Off,
    Local,
    Url,
}

/// Config tab fields. Defaults match design §7.2 Darwin skeleton (also parses on Linux).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Overlay recognized keys onto `FormState::default()`. Unknown keys are ignored.
/// Returns an error if `yaml` is not a mapping (or does not parse).
pub fn from_yaml(yaml: &str) -> Result<FormState> {
    let root: serde_yaml::Value = serde_yaml::from_str(yaml).context("invalid yaml")?;
    if !root.is_mapping() {
        anyhow::bail!("yaml is not a mapping");
    }
    let mut form = FormState::default();
    overlay_form(&mut form, &root);
    if advanced_present(&form) {
        form.advanced_expanded = true;
    }
    Ok(form)
}

/// Read `path` into a form. Missing file → `FormState::default()`.
/// Unreadable or invalid yaml → `Err` without creating, deleting, or rewriting `path`.
pub fn load_from_path(path: &Path) -> Result<FormState> {
    match std::fs::read_to_string(path) {
        Ok(text) => from_yaml(&text).with_context(|| format!("parse {}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(FormState::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// On-disk route cache. Only these three keys are written; unknown keys are ignored on read.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TuiJson {
    #[serde(default)]
    route_mode: String,
    #[serde(default)]
    route_url: String,
    #[serde(default)]
    route_local: String,
}

fn route_mode_wire(mode: RouteMode) -> &'static str {
    match mode {
        RouteMode::Off => "off",
        RouteMode::Local => "local",
        RouteMode::Url => "url",
    }
}

fn parse_route_mode(raw: &str) -> RouteMode {
    match raw.trim() {
        "local" => RouteMode::Local,
        "url" => RouteMode::Url,
        _ => RouteMode::Off,
    }
}

/// Overlay `routeMode` / `routeUrl` / `routeLocal` from `path` onto `form`.
///
/// Missing file → no-op (`Ok`). Unreadable or invalid JSON → `Err` without
/// creating, deleting, or rewriting `path`. When the file exists, its three
/// fields win over yaml (including handwritten `route.file`).
pub fn overlay_tui_json(form: &mut FormState, path: &Path) -> Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    if !value.is_object() {
        anyhow::bail!("tui.json is not an object");
    }
    let parsed: TuiJson =
        serde_json::from_value(value).with_context(|| format!("parse {}", path.display()))?;
    form.route_mode = parse_route_mode(&parsed.route_mode);
    form.route_url = parsed.route_url;
    form.route_local_path = parsed.route_local;
    Ok(())
}

fn overlay_form(form: &mut FormState, root: &serde_yaml::Value) {
    if let Some(s) = root.get("server").and_then(yaml_string) {
        apply_server(form, &s);
    }
    if let Some(s) = root.get("auth").and_then(yaml_string) {
        form.auth = s;
    }
    if let Some(tls) = root.get("tls").filter(|v| v.is_mapping()) {
        if let Some(s) = tls.get("sni").and_then(yaml_string) {
            form.tls_sni = s;
        }
        form.verify_cert = tls.get("insecure").and_then(serde_yaml::Value::as_bool) != Some(true);
    }
    if let Some(tun) = root.get("tun").filter(|v| v.is_mapping()) {
        if let Some(s) = tun.get("name").and_then(yaml_string) {
            form.tun_name = s;
        }
        if let Some(s) = tun.get("timeout").and_then(yaml_string) {
            form.timeout = s;
        }
        if let Some(addr) = tun.get("address").filter(|v| v.is_mapping()) {
            if let Some(s) = addr.get("ipv4").and_then(yaml_string) {
                form.tun_ipv4 = s;
            }
            if let Some(s) = addr.get("ipv6").and_then(yaml_string) {
                form.tun_ipv6 = s;
            }
        }
        if let Some(route) = tun.get("route") {
            form.write_route = true;
            if let Some(ex) = route.get("ipv4Exclude").and_then(first_list_string) {
                form.ipv4_exclude = ex;
            }
        } else {
            form.write_route = false;
        }
    }
    if let Some(file) = root
        .get("route")
        .and_then(|r| r.get("file"))
        .and_then(yaml_string)
    {
        let file = file.trim();
        if !file.is_empty() {
            form.route_mode = RouteMode::Local;
            form.route_local_path = file.to_string();
        }
    }
    if let Some(bw) = root.get("bandwidth").filter(|v| v.is_mapping()) {
        if let Some(s) = bw.get("up").and_then(yaml_string) {
            form.bandwidth_up = s;
        }
        if let Some(s) = bw.get("down").and_then(yaml_string) {
            form.bandwidth_down = s;
        }
    }
    if let Some(obfs) = root.get("obfs").filter(|v| v.is_mapping()) {
        if let Some(s) = obfs.get("type").and_then(yaml_string) {
            form.obfs_type = s;
        }
        let ty = if form.obfs_type.trim().is_empty() {
            "salamander"
        } else {
            form.obfs_type.trim()
        };
        if let Some(pw) = obfs
            .get(ty)
            .and_then(|block| block.get("password"))
            .and_then(yaml_string)
        {
            form.obfs_password = pw;
        }
    }
    if let Some(quic) = root.get("quic").filter(|v| v.is_mapping()) {
        if let Some(s) = quic.get("initStreamReceiveWindow").and_then(yaml_string) {
            form.quic_init_stream_window = s;
        }
        if let Some(s) = quic.get("maxStreamReceiveWindow").and_then(yaml_string) {
            form.quic_max_stream_window = s;
        }
        if let Some(s) = quic.get("initConnReceiveWindow").and_then(yaml_string) {
            form.quic_init_conn_window = s;
        }
        if let Some(s) = quic.get("maxConnReceiveWindow").and_then(yaml_string) {
            form.quic_max_conn_window = s;
        }
    }
    if let Some(b) = root.get("lazy").and_then(serde_yaml::Value::as_bool) {
        form.lazy = b;
    }
    if let Some(b) = root.get("fastOpen").and_then(serde_yaml::Value::as_bool) {
        form.fast_open = b;
    }
    if let Some(s) = root
        .get("socks5")
        .and_then(|s| s.get("listen"))
        .and_then(yaml_string)
    {
        form.socks5_listen = s;
    }
    if let Some(s) = root
        .get("transport")
        .and_then(|t| t.get("udp"))
        .and_then(|u| u.get("hopInterval"))
        .and_then(yaml_string)
    {
        form.hop_interval = s;
    }
}

fn apply_server(form: &mut FormState, raw: &str) {
    let raw = raw.trim();
    if let Some((head, rest)) = raw.split_once(',') {
        form.server = head.trim().to_string();
        form.hop_ports = rest.trim().to_string();
    } else {
        form.server = raw.to_string();
    }
}

fn advanced_present(form: &FormState) -> bool {
    !form.bandwidth_up.trim().is_empty()
        || !form.bandwidth_down.trim().is_empty()
        || !form.obfs_type.trim().is_empty()
        || !form.obfs_password.trim().is_empty()
        || !form.hop_ports.trim().is_empty()
        || !form.hop_interval.trim().is_empty()
        || !form.quic_init_stream_window.trim().is_empty()
        || !form.quic_max_stream_window.trim().is_empty()
        || !form.quic_init_conn_window.trim().is_empty()
        || !form.quic_max_conn_window.trim().is_empty()
        || form.lazy
        || form.fast_open
        || !form.socks5_listen.trim().is_empty()
        || !form.tun_ipv6.trim().is_empty()
}

fn yaml_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn first_list_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::Sequence(seq) => seq.first().and_then(yaml_string),
        other => yaml_string(other),
    }
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
    write_tui_json(form, &hy_dir).await?;
    Ok(dest)
}

/// Atomic write of `hy_dir/tui.json` (0600). Only routeMode / routeUrl / routeLocal.
async fn write_tui_json(form: &FormState, hy_dir: &Path) -> Result<()> {
    let body = TuiJson {
        route_mode: route_mode_wire(form.route_mode).to_string(),
        route_url: form.route_url.clone(),
        route_local: form.route_local_path.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&body).context("serialize tui.json")?;
    let dest = hy_dir.join("tui.json");
    let tmp = hy_dir.join(format!(".tui.json.{}.tmp", std::process::id()));
    tokio::fs::write(&tmp, &bytes)
        .await
        .with_context(|| format!("write {}", tmp.display()))?;
    set_mode(&tmp, 0o600)?;
    tokio::fs::rename(&tmp, &dest)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    set_mode(&dest, 0o600)?;
    Ok(())
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

    fn filled_form() -> FormState {
        let mut form = FormState::default();
        form.server = "10.1.2.3:443".into();
        form.auth = "hunter2".into();
        form.tls_sni = "example.com".into();
        form.verify_cert = true;
        form.tun_name = "hy0".into();
        form.tun_ipv4 = "10.0.0.2/30".into();
        form.write_route = true;
        form.ipv4_exclude = "10.1.2.3/32".into();
        form.timeout = "30s".into();
        form.advanced_expanded = true;
        form.socks5_listen = "127.0.0.1:1080".into();
        form.bandwidth_up = "100mbps".into();
        form.bandwidth_down = "500mbps".into();
        form.hop_ports = "10000-20000".into();
        form
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("hy-tui-w1-{tag}-{nanos}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_path_and_empty_home_are_defaults() {
        let missing = PathBuf::from("/tmp/hy-tui-w1-missing/no-such/client.yaml");
        assert_eq!(load_from_path(&missing).unwrap(), FormState::default());

        let home = scratch_dir("empty-home");
        let path = home.join(".hy").join("client.yaml");
        assert!(!path.exists());
        assert_eq!(load_from_path(&path).unwrap(), FormState::default());
        assert!(!path.exists(), "missing path must not create client.yaml");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn from_yaml_roundtrip_filled_form() {
        let form = filled_form();
        let loaded = from_yaml(&to_yaml(&form)).expect("from_yaml");
        assert_eq!(loaded.server, form.server);
        assert_eq!(loaded.auth, form.auth);
        assert_eq!(loaded.tls_sni, form.tls_sni);
        assert_eq!(loaded.verify_cert, form.verify_cert);
        assert_eq!(loaded.tun_name, form.tun_name);
        assert_eq!(loaded.tun_ipv4, form.tun_ipv4);
        assert_eq!(loaded.write_route, form.write_route);
        assert_eq!(loaded.ipv4_exclude, form.ipv4_exclude);
        assert_eq!(loaded.timeout, form.timeout);
        assert_eq!(loaded.socks5_listen, form.socks5_listen);
        assert_eq!(loaded.bandwidth_up, form.bandwidth_up);
        assert_eq!(loaded.bandwidth_down, form.bandwidth_down);
        assert_eq!(loaded.hop_ports, form.hop_ports);
        assert!(loaded.advanced_expanded);
    }

    #[test]
    fn bad_yaml_returns_error_and_does_not_delete_fixture() {
        assert!(from_yaml("this is garbage").is_err());
        assert!(from_yaml("[]").is_err());
        assert!(from_yaml("- just\n- a\n- list\n").is_err());

        let dir = scratch_dir("bad-yaml");
        let path = dir.join("client.yaml");
        let original = "this is garbage\nnot: [ a: mapping: :\n";
        std::fs::write(&path, original).unwrap();
        assert!(load_from_path(&path).is_err());
        assert!(path.is_file(), "bad yaml must not delete the file");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "bad yaml must leave contents unchanged"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tls_insecure_maps_to_verify_cert() {
        let insecure = from_yaml("tls:\n  insecure: true\n").unwrap();
        assert!(!insecure.verify_cert);

        let omitted = from_yaml("tls:\n  sni: example.com\n").unwrap();
        assert!(omitted.verify_cert);
        assert_eq!(omitted.tls_sni, "example.com");

        let off = from_yaml("tls:\n  insecure: false\n").unwrap();
        assert!(off.verify_cert);
    }

    #[test]
    fn no_tun_route_sets_write_route_false() {
        let form = from_yaml("tun:\n  name: hy0\n  timeout: 45s\n").unwrap();
        assert!(!form.write_route);
        assert_eq!(form.tun_name, "hy0");
        assert_eq!(form.timeout, "45s");
        assert_eq!(form.ipv4_exclude, FormState::default().ipv4_exclude);
    }

    #[test]
    fn handwritten_route_file_sets_local_mode() {
        let form = from_yaml("route:\n  file: /tmp/x.conf\n").unwrap();
        assert_eq!(form.route_mode, RouteMode::Local);
        assert_eq!(form.route_local_path, "/tmp/x.conf");
        let yaml = to_yaml(&form);
        let v = value(&yaml);
        assert!(
            v.get("route").is_none(),
            "Save must not emit route.file\n{yaml}"
        );
        assert!(!yaml.contains("/tmp/x.conf"), "{yaml}");
    }

    #[test]
    fn unknown_keys_ignored_known_keys_load() {
        let yaml = r#"
server: 10.9.8.7:443
auth: s3cret
madeUp: true
tls:
  sni: sni.example
  pinSHA256: deadbeef
tun:
  name: hy0
  mystery: 1
  address:
    ipv4: 10.0.0.2/30
extraTop:
  nested: 1
"#;
        let form = from_yaml(yaml).unwrap();
        assert_eq!(form.server, "10.9.8.7:443");
        assert_eq!(form.auth, "s3cret");
        assert_eq!(form.tls_sni, "sni.example");
        assert_eq!(form.tun_name, "hy0");
        assert_eq!(form.tun_ipv4, "10.0.0.2/30");
        assert!(!form.write_route);
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

    fn load_saved(home: &Path) -> FormState {
        let mut form = load_from_path(&home.join(".hy").join("client.yaml")).unwrap();
        overlay_tui_json(&mut form, &home.join(".hy").join("tui.json")).unwrap();
        form
    }

    #[tokio::test]
    async fn save_to_writes_tui_json_0600_route_keys_only() {
        let home = scratch_dir("tui-json-shape");
        let mut form = FormState::default();
        form.auth = "must-not-appear-in-tui-json".into();
        form.hy_path = "/opt/custom/hy".into();
        form.route_mode = RouteMode::Url;
        form.route_url = "https://example.com/rules.conf".into();
        save_to(&form, &home).await.expect("save_to");

        let yaml_path = home.join(".hy").join("client.yaml");
        let tui_path = home.join(".hy").join("tui.json");
        assert!(yaml_path.is_file(), "client.yaml next to tui.json");
        assert!(tui_path.is_file());
        let mode = std::fs::metadata(&tui_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "tui.json must be 0600");

        let text = std::fs::read_to_string(&tui_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        let obj = v.as_object().expect("object");
        let mut keys: Vec<_> = obj.keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["routeLocal", "routeMode", "routeUrl"],
            "tui.json keys: {text}"
        );
        assert_eq!(obj.get("routeMode").and_then(|x| x.as_str()), Some("url"));
        assert_eq!(
            obj.get("routeUrl").and_then(|x| x.as_str()),
            Some("https://example.com/rules.conf")
        );
        assert_eq!(obj.get("routeLocal").and_then(|x| x.as_str()), Some(""));
        assert!(!text.contains("auth"), "{text}");
        assert!(!text.contains("must-not-appear-in-tui-json"), "{text}");
        assert!(!text.contains("/opt/custom/hy"), "{text}");
        assert!(
            !text.contains("hy_path") && !text.contains("hyPath"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn save_url_mode_loads_back() {
        let home = scratch_dir("save-url");
        let mut form = FormState::default();
        form.route_mode = RouteMode::Url;
        form.route_url = "https://example.com/rules.conf".into();
        form.route_local_path.clear();
        save_to(&form, &home).await.unwrap();
        let loaded = load_saved(&home);
        assert_eq!(loaded.route_mode, RouteMode::Url);
        assert_eq!(loaded.route_url, "https://example.com/rules.conf");
        assert_eq!(loaded.route_local_path, "");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn save_local_mode_loads_back() {
        let home = scratch_dir("save-local");
        let mut form = FormState::default();
        form.route_mode = RouteMode::Local;
        form.route_local_path = "/tmp/my-route.conf".into();
        save_to(&form, &home).await.unwrap();
        let loaded = load_saved(&home);
        assert_eq!(loaded.route_mode, RouteMode::Local);
        assert_eq!(loaded.route_local_path, "/tmp/my-route.conf");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn save_off_mode_loads_back() {
        let home = scratch_dir("save-off");
        let mut form = FormState::default();
        form.route_mode = RouteMode::Off;
        form.route_url.clear();
        form.route_local_path.clear();
        save_to(&form, &home).await.unwrap();
        let loaded = load_saved(&home);
        assert_eq!(loaded.route_mode, RouteMode::Off);
        assert_eq!(loaded.route_url, "");
        assert_eq!(loaded.route_local_path, "");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn yaml_route_file_without_tui_json_stays_local() {
        let home = scratch_dir("yaml-route-no-tui");
        let hy = home.join(".hy");
        std::fs::create_dir_all(&hy).unwrap();
        std::fs::write(
            hy.join("client.yaml"),
            "server: 10.1.2.3:443\nauth: secret\nroute:\n  file: /tmp/x.conf\n",
        )
        .unwrap();
        assert!(!hy.join("tui.json").exists());
        let mut form = load_from_path(&hy.join("client.yaml")).unwrap();
        overlay_tui_json(&mut form, &hy.join("tui.json")).unwrap();
        assert_eq!(form.route_mode, RouteMode::Local);
        assert_eq!(form.route_local_path, "/tmp/x.conf");
        assert!(
            !hy.join("tui.json").exists(),
            "load must not create tui.json"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tui_json_wins_over_yaml_route_file() {
        let home = scratch_dir("tui-wins");
        let hy = home.join(".hy");
        std::fs::create_dir_all(&hy).unwrap();
        std::fs::write(hy.join("client.yaml"), "route:\n  file: /tmp/x.conf\n").unwrap();

        std::fs::write(
            hy.join("tui.json"),
            r#"{"routeMode":"off","routeUrl":"","routeLocal":""}"#,
        )
        .unwrap();
        let mut form = load_from_path(&hy.join("client.yaml")).unwrap();
        overlay_tui_json(&mut form, &hy.join("tui.json")).unwrap();
        assert_eq!(form.route_mode, RouteMode::Off);
        assert_eq!(form.route_local_path, "");

        std::fs::write(
            hy.join("tui.json"),
            r#"{"routeMode":"url","routeUrl":"https://example.com/rules.conf","routeLocal":""}"#,
        )
        .unwrap();
        let mut form = load_from_path(&hy.join("client.yaml")).unwrap();
        overlay_tui_json(&mut form, &hy.join("tui.json")).unwrap();
        assert_eq!(form.route_mode, RouteMode::Url);
        assert_eq!(form.route_url, "https://example.com/rules.conf");
        assert_eq!(form.route_local_path, "");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn bad_tui_json_returns_error_and_leaves_bytes() {
        let home = scratch_dir("bad-tui");
        let hy = home.join(".hy");
        std::fs::create_dir_all(&hy).unwrap();
        let path = hy.join("tui.json");
        for original in ["this is garbage\n", "[]\n", "\"not-an-object\"\n"] {
            std::fs::write(&path, original).unwrap();
            let mut form = FormState::default();
            form.route_mode = RouteMode::Local;
            form.route_local_path = "/keep-me.conf".into();
            assert!(overlay_tui_json(&mut form, &path).is_err(), "{original:?}");
            assert!(path.is_file(), "bad tui.json must not be deleted");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                original,
                "bad tui.json must leave contents unchanged"
            );
            assert_eq!(form.route_mode, RouteMode::Local);
            assert_eq!(form.route_local_path, "/keep-me.conf");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn load_only_does_not_create_tui_json() {
        let home = scratch_dir("load-only");
        let yaml = home.join(".hy").join("client.yaml");
        let tui = home.join(".hy").join("tui.json");
        assert!(!yaml.exists());
        assert!(!tui.exists());
        let mut form = load_from_path(&yaml).unwrap();
        overlay_tui_json(&mut form, &tui).unwrap();
        assert_eq!(form, FormState::default());
        assert!(!yaml.exists(), "load must not create client.yaml");
        assert!(!tui.exists(), "load must not create tui.json");
        assert!(!home.join(".hy").exists(), "load must not create .hy");
        let _ = std::fs::remove_dir_all(&home);
    }
}
