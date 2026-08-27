//! Download a Shadowrocket-style rules file over HTTPS into `~/.hy/route.conf`.
//!
//! The URL is treated as data only: the file is never executed. HTTPS only;
//! a small number of redirects; 8 MiB cap. After write, a RULE-SET-heavy body
//! (lazy.conf style) sets a warning flag so the UI can say the rules will not
//! work in hy, without blocking Start.

use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

const USER_AGENT: &str = "hy-tui/0.1 (+https://github.com/wusir27/hy-tool)";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: u32 = 5;

/// Hard cap on a downloaded (or installed) rules file.
pub const MAX_ROUTE_BYTES: u64 = 8 * 1024 * 1024;

/// Shown in Config status when the body is almost all `RULE-SET` (hy skips those lines).
pub const RULESET_UNUSABLE: &str = "这份规则 hy 用不了";

/// Result of writing `~/.hy/route.conf`.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub path: PathBuf,
    pub ruleset_warning: bool,
}

pub fn default_route_conf(home: &Path) -> PathBuf {
    home.join(".hy").join("route.conf")
}

/// Fetch `url` (HTTPS only) and atomically install as `home/.hy/route.conf` (0600).
pub fn fetch_to_home(home: &Path, url: &str) -> Result<FetchResult> {
    let bytes = download_https(url)?;
    install_bytes(home, &bytes)
}

/// Download the URL body. Rejects `http://`. Does not write any dest file.
pub fn download_https(url: &str) -> Result<Vec<u8>> {
    let url = url.trim();
    require_https(url)?;

    let agent = ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout(DOWNLOAD_TIMEOUT)
        .https_only(true)
        .redirects(MAX_REDIRECTS)
        .build();

    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(e) => return Err(map_ureq(e)),
    };
    let status = resp.status();
    if status == 404 {
        bail!("download 404: {url}");
    }
    if !(200..300).contains(&status) {
        bail!("download HTTP {status}: {url}");
    }
    if let Some(len) = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok()) {
        if len > MAX_ROUTE_BYTES {
            bail!("route file exceeds {MAX_ROUTE_BYTES} bytes");
        }
    }

    read_capped(resp.into_reader(), MAX_ROUTE_BYTES)
        .with_context(|| format!("read {url}"))
}

fn require_https(url: &str) -> Result<()> {
    if url.is_empty() {
        bail!("route URL is empty");
    }
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("");
    if scheme.eq_ignore_ascii_case("https") {
        return Ok(());
    }
    if scheme.eq_ignore_ascii_case("http") {
        bail!("HTTPS only: http:// is not allowed");
    }
    bail!("HTTPS only");
}

fn read_capped(reader: impl Read, max: u64) -> Result<Vec<u8>> {
    let mut reader = reader.take(max.saturating_add(1));
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        bail!("route file exceeds {max} bytes");
    }
    Ok(buf)
}

/// Write `bytes` to `home/.hy/route.conf` via temp+rename (0600). Creates `~/.hy` (0700).
///
/// Oversize bodies error and do not write dest. The file is never marked executable.
pub fn install_bytes(home: &Path, bytes: &[u8]) -> Result<FetchResult> {
    if bytes.len() as u64 > MAX_ROUTE_BYTES {
        bail!("route file exceeds {MAX_ROUTE_BYTES} bytes");
    }
    let hy_dir = home.join(".hy");
    ensure_dir(&hy_dir, 0o700)?;

    let dest = default_route_conf(home);
    let tmp = hy_dir.join(format!(".route.conf.{}.tmp", std::process::id()));
    let _cleanup = TmpCleanup(&tmp);

    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, &dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    set_mode(&dest, 0o600)?;

    let text = String::from_utf8_lossy(bytes);
    Ok(FetchResult {
        path: dest,
        ruleset_warning: ruleset_heavy(&text),
    })
}

/// True when nearly all actionable rule lines are `RULE-SET` (comments/blank ignored).
///
/// Actionable = Shadowrocket rule rows (`TYPE,...`). `[General]` assignments are skipped.
pub fn ruleset_heavy(body: &str) -> bool {
    let mut actionable = 0u64;
    let mut ruleset = 0u64;
    for raw in body.lines() {
        let line = strip_inline_comment(raw.trim());
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some(kind) = rule_kind(line) else {
            continue;
        };
        actionable += 1;
        if kind.eq_ignore_ascii_case("RULE-SET") {
            ruleset += 1;
        }
    }
    if actionable == 0 || ruleset == 0 {
        return false;
    }
    // ≥ 80% of rule rows are RULE-SET ("nearly all", still true for lazy.conf
    // which keeps a few DOMAIN-SUFFIX / GEOIP / FINAL lines).
    ruleset.saturating_mul(5) >= actionable.saturating_mul(4)
}

fn strip_inline_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => line[..i].trim_end(),
        None => line,
    }
}

fn rule_kind(line: &str) -> Option<&str> {
    let first = line.split(',').next()?.trim();
    if first.is_empty() || first.contains('=') {
        return None;
    }
    if !first
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    if !first.as_bytes().first()?.is_ascii_alphabetic() {
        return None;
    }
    Some(first)
}

struct TmpCleanup<'a>(&'a Path);

impl Drop for TmpCleanup<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

fn ensure_dir(path: &Path, mode: u32) -> Result<()> {
    match fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_dir() {
                bail!("{} exists and is not a directory", path.display());
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
            set_mode(path, mode)?;
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("stat {}", path.display())),
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .with_context(|| format!("metadata {}", path.display()))?
        .permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms)
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))?;
    Ok(())
}

fn map_ureq(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Status(404, _) => anyhow::anyhow!("download 404"),
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let body = body.trim();
            if body.is_empty() {
                anyhow::anyhow!("HTTP {code}")
            } else {
                anyhow::anyhow!("HTTP {code}: {body}")
            }
        }
        ureq::Error::Transport(t) => {
            let msg = t.to_string();
            if msg.to_ascii_lowercase().contains("https_only")
                || msg.to_ascii_lowercase().contains("non https")
            {
                anyhow::anyhow!("HTTPS only: {msg}")
            } else {
                anyhow::anyhow!("network error: {t}")
            }
        }
    }
}

#[cfg(test)]
pub(crate) const LAZY_FIXTURE: &str = "\
# lazy-like: almost all RULE-SET
[General]
bypass-system = true
skip-proxy = 192.168.0.0/16, 10.0.0.0/8

[Rule]
RULE-SET,https://example.com/ai.txt,PROXY
RULE-SET,https://example.com/youtube.list,PROXY
RULE-SET,https://example.com/netflix.list,PROXY
RULE-SET,https://example.com/telegram.list,PROXY
RULE-SET,https://example.com/github.list,PROXY
RULE-SET,https://example.com/google.list,PROXY
RULE-SET,https://example.com/china.list,DIRECT
RULE-SET,https://example.com/lan.list,DIRECT
GEOIP,CN,DIRECT
FINAL,PROXY
";

#[cfg(test)]
pub(crate) const SR_CNIP_FIXTURE: &str = "\
# sr_cnip-like: suffix / CIDR
[General]
bypass-system = true

[Rule]
DOMAIN-SUFFIX,tedcdn.com,PROXY
DOMAIN-SUFFIX,telegram.org,PROXY
DOMAIN-SUFFIX,t.me,PROXY
IP-CIDR,91.108.56.0/22,PROXY
IP-CIDR,149.154.160.0/20,PROXY
DOMAIN-SUFFIX,github.com,PROXY
GEOIP,CN,DIRECT
FINAL,proxy
";

#[cfg(test)]
pub(crate) fn temp_home(tag: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "hy-tui-route-{tag}-{nanos}-{}",
        std::process::id()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn https_only_http_rejected_does_not_write() {
        let home = temp_home("http");
        let dest = default_route_conf(&home);
        let err = fetch_to_home(&home, "http://example.com/sr_cnip.conf").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_ascii_lowercase().contains("https"),
            "expected HTTPS-only error, got {msg}"
        );
        assert!(!dest.exists(), "http:// must not write {}", dest.display());
        assert!(
            !home.join(".hy").exists(),
            "must not create ~/.hy on rejected URL"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn https_only_rejects_http_uppercase_scheme() {
        let err = download_https("HTTP://example.com/x.conf").unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("https"),
            "{}",
            err
        );
    }

    #[test]
    fn size_cap_rejected_dest_not_written() {
        let home = temp_home("oversize");
        let dest = default_route_conf(&home);
        let too_big = vec![b'x'; (MAX_ROUTE_BYTES as usize) + 1];
        let err = install_bytes(&home, &too_big).unwrap_err();
        assert!(
            err.to_string().contains(&MAX_ROUTE_BYTES.to_string()),
            "{}",
            err
        );
        assert!(!dest.exists(), "oversize must not write {}", dest.display());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn size_cap_leaves_previous_dest() {
        let home = temp_home("keep-good");
        let good = install_bytes(&home, SR_CNIP_FIXTURE.as_bytes()).unwrap();
        let too_big = vec![b'y'; (MAX_ROUTE_BYTES as usize) + 1];
        assert!(install_bytes(&home, &too_big).is_err());
        assert_eq!(fs::read(&good.path).unwrap(), SR_CNIP_FIXTURE.as_bytes());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn fixture_write_temp_home_mode_0600() {
        let home = temp_home("0600");
        let result = install_bytes(&home, SR_CNIP_FIXTURE.as_bytes()).unwrap();
        let dest = default_route_conf(&home);
        assert_eq!(result.path, dest);
        assert!(dest.is_file());
        assert_eq!(fs::read(&dest).unwrap(), SR_CNIP_FIXTURE.as_bytes());
        let file_mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "route.conf must be 0600, got {file_mode:o}");
        let dir_mode = fs::metadata(home.join(".hy"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, ".hy must be 0700 when created");
        let exec = file_mode & 0o111;
        assert_eq!(exec, 0, "route.conf must not be executable");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn ruleset_heavy_lazy_fixture_warns() {
        assert!(ruleset_heavy(LAZY_FIXTURE));
        let home = temp_home("lazy");
        let result = install_bytes(&home, LAZY_FIXTURE.as_bytes()).unwrap();
        assert!(result.ruleset_warning);
        assert_eq!(RULESET_UNUSABLE, "这份规则 hy 用不了");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn suffix_cidr_fixture_no_warning() {
        assert!(!ruleset_heavy(SR_CNIP_FIXTURE));
        let home = temp_home("sr");
        let result = install_bytes(&home, SR_CNIP_FIXTURE.as_bytes()).unwrap();
        assert!(!result.ruleset_warning);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn comments_and_blank_ignored() {
        let body = "\n# comment\n\n  \nDOMAIN-SUFFIX,a.com,PROXY\n";
        assert!(!ruleset_heavy(body));
        let only_comments = "# hi\n\n[Rule]\n# still comment\n";
        assert!(!ruleset_heavy(only_comments));
    }

    /// Live GitHub raw fetch. Skips cleanly when the network is unavailable.
    #[test]
    fn live_download_sr_cnip() {
        const URLS: &[&str] = &[
            "https://raw.githubusercontent.com/Johnshall/Shadowrocket-ADBlock-Rules-Forever/release/sr_cnip.conf",
            "https://raw.githubusercontent.com/Johnshall/Shadowrocket-ADBlock-Rules-Forever/main/sr_cnip.conf",
        ];
        let home = temp_home("live-sr");
        let mut last_err = None;
        for url in URLS {
            match fetch_to_home(&home, url) {
                Ok(result) => {
                    assert!(result.path.is_file(), "{}", result.path.display());
                    assert_eq!(result.path, default_route_conf(&home));
                    let mode = fs::metadata(&result.path).unwrap().permissions().mode() & 0o777;
                    assert_eq!(mode, 0o600);
                    let body = fs::read_to_string(&result.path).unwrap();
                    assert!(
                        body.contains("DOMAIN-SUFFIX") || body.contains("IP-CIDR"),
                        "sr_cnip body should have suffix/CIDR rules"
                    );
                    assert!(
                        !result.ruleset_warning,
                        "sr_cnip-like body must not warn RULE-SET"
                    );
                    let _ = fs::remove_dir_all(&home);
                    return;
                }
                Err(e) => last_err = Some(e),
            }
        }
        eprintln!(
            "skip live sr_cnip: {}",
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unreachable".into())
        );
        let _ = fs::remove_dir_all(&home);
    }

    /// Live lazy.conf, or the in-tree fixture if the network fails.
    #[test]
    fn live_lazy_or_fixture_triggers_warning() {
        const URL: &str = "https://raw.githubusercontent.com/Johnshall/Shadowrocket-ADBlock-Rules-Forever/release/lazy.conf";
        let home = temp_home("live-lazy");
        let result = match fetch_to_home(&home, URL) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skip live lazy ({e}); using fixture");
                install_bytes(&home, LAZY_FIXTURE.as_bytes()).unwrap()
            }
        };
        assert!(
            result.ruleset_warning,
            "lazy-like body must set the RULE-SET warning"
        );
        assert!(result.path.is_file());
        let _ = fs::remove_dir_all(&home);
    }
}
