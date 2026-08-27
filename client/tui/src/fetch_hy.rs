//! Download the matching hy Release asset, verify SHA256SUMS, install to ~/.hy/bin/hy.
//!
//! Network I/O is synchronous (ureq). Call from `tokio::task::spawn_blocking` in the UI.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::detect;

const RELEASES_LATEST: &str = "https://api.github.com/repos/wusir27/hy/releases/latest";
const USER_AGENT: &str = "hy-tui/0.1 (+https://github.com/wusir27/hy-tool)";
const API_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SUMS_BYTES: u64 = 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 80 * 1024 * 1024;

/// Result of a verified install into `home/.hy/bin/hy`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct InstallResult {
    pub path: PathBuf,
    pub tag: String,
    pub asset: String,
}

pub fn default_hy_bin(home: &Path) -> PathBuf {
    home.join(".hy").join("bin").join("hy")
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Parse GNU (`<hex>  name`) or BSD (`SHA256 (name) = <hex>`) SHA256SUMS text.
pub fn parse_sha256sums(text: &str) -> Result<HashMap<String, [u8; 32]>> {
    let mut map = HashMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lineno = i + 1;
        let (name, hex) = parse_sums_line(line)
            .with_context(|| format!("SHA256SUMS line {lineno}"))?;
        let digest = parse_hex32(&hex)
            .with_context(|| format!("SHA256SUMS line {lineno} hash"))?;
        map.insert(name, digest);
    }
    if map.is_empty() {
        bail!("SHA256SUMS is empty");
    }
    Ok(map)
}

fn parse_sums_line(line: &str) -> Result<(String, String)> {
    if let Some(rest) = line.strip_prefix("SHA256 (") {
        let (name, rest) = rest
            .split_once(')')
            .ok_or_else(|| anyhow::anyhow!("BSD SHA256 line missing ')'"))?;
        let hex = rest
            .trim()
            .strip_prefix('=')
            .unwrap_or(rest)
            .trim()
            .to_string();
        return Ok((basename(name), hex));
    }
    let mut parts = line.split_whitespace();
    let hex = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing hash"))?
        .to_string();
    let name = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing filename"))?;
    let name = name.strip_prefix('*').unwrap_or(name);
    Ok((basename(name), hex))
}

fn basename(name: &str) -> String {
    name.trim()
        .trim_start_matches("./")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_string()
}

fn parse_hex32(s: &str) -> Result<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("expected 64 hex digits, got {:?}", s);
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}

/// Check `bytes` against the SUMS line for `filename`. Never returns Ok on mismatch.
pub fn verify_bytes(bytes: &[u8], sums: &str, filename: &str) -> Result<[u8; 32]> {
    let map = parse_sha256sums(sums)?;
    let expected = map.get(filename).ok_or_else(|| {
        anyhow::anyhow!("SHA256SUMS has no entry for {filename}")
    })?;
    let got = sha256(bytes);
    if got.as_ref() != expected.as_ref() {
        bail!("SHA256 mismatch for {filename}");
    }
    Ok(got)
}

/// Verify then atomically install as `home/.hy/bin/hy` (0755). Creates `~/.hy/bin` (0700).
///
/// On hash failure the destination is not written (a previous good file is left intact).
pub fn install_verified(
    home: &Path,
    asset_name: &str,
    bytes: &[u8],
    sums: &str,
) -> Result<PathBuf> {
    verify_bytes(bytes, sums, asset_name)?;
    write_hy_bin(home, bytes)
}

fn write_hy_bin(home: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let hy_dir = home.join(".hy");
    let bin_dir = hy_dir.join("bin");
    ensure_dir(&hy_dir, 0o700)?;
    ensure_dir(&bin_dir, 0o700)?;

    let dest = bin_dir.join("hy");
    let tmp = bin_dir.join(format!(".hy.{}.tmp", std::process::id()));
    let _cleanup = TmpCleanup(&tmp);

    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    set_mode(&tmp, 0o755)?;
    fs::rename(&tmp, &dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    set_mode(&dest, 0o755)?;
    Ok(dest)
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

/// Fetch GitHub latest release, download SHA256SUMS + host asset, verify, install.
pub fn install_latest(home: &Path) -> Result<InstallResult> {
    let asset = detect::host_asset_name()?;
    install_latest_asset(home, asset)
}

/// Same as [`install_latest`] but the asset name is explicit (tests / live linux-amd64).
pub fn install_latest_asset(home: &Path, asset: &str) -> Result<InstallResult> {
    let release = fetch_latest_release()?;
    let sums_url = asset_url(&release, "SHA256SUMS")
        .ok_or_else(|| anyhow::anyhow!("release {} has no SHA256SUMS", release.tag_name))?;
    let bin_url = asset_url(&release, asset)
        .ok_or_else(|| anyhow::anyhow!("no asset for this triple"))?;

    let sums = download_text(&sums_url)?;
    let bytes = download_bytes(&bin_url, MAX_ASSET_BYTES)?;
    let path = install_verified(home, asset, &bytes, &sums)?;
    Ok(InstallResult {
        path,
        tag: release.tag_name,
        asset: asset.to_string(),
    })
}

#[derive(Debug, serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, serde::Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn asset_url(release: &Release, name: &str) -> Option<String> {
    release
        .assets
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.browser_download_url.clone())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
}

fn fetch_latest_release() -> Result<Release> {
    let resp = match agent(API_TIMEOUT).get(RELEASES_LATEST).call() {
        Ok(r) => r,
        Err(e) => return Err(map_ureq(e)),
    };
    let status = resp.status();
    if status == 404 {
        bail!("GitHub release not found (404)");
    }
    if !(200..300).contains(&status) {
        bail!("GitHub API HTTP {status}");
    }
    serde_json::from_reader(resp.into_reader()).context("parse GitHub latest release JSON")
}

fn download_text(url: &str) -> Result<String> {
    let bytes = download_bytes(url, MAX_SUMS_BYTES)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn download_bytes(url: &str, max: u64) -> Result<Vec<u8>> {
    let resp = match agent(DOWNLOAD_TIMEOUT).get(url).call() {
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
    let mut reader = resp.into_reader().take(max.saturating_add(1));
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .with_context(|| format!("read {url}"))?;
    if buf.len() as u64 > max {
        bail!("download exceeded {max} bytes: {url}");
    }
    Ok(buf)
}

fn map_ureq(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Status(404, _) => anyhow::anyhow!("GitHub release not found (404)"),
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let body = body.trim();
            if body.is_empty() {
                anyhow::anyhow!("HTTP {code}")
            } else {
                anyhow::anyhow!("HTTP {code}: {body}")
            }
        }
        ureq::Error::Transport(t) => anyhow::anyhow!("network error: {t}"),
    }
}

/// True when the GitHub latest-release API answers 2xx quickly.
#[cfg(test)]
fn github_reachable() -> bool {
    match agent(Duration::from_secs(5)).get(RELEASES_LATEST).call() {
        Ok(r) => (200..300).contains(&r.status()),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn hex32(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn fixture_bytes() -> &'static [u8] {
        b"hy-tui fixture binary\n"
    }

    fn gnu_sums_for(filename: &str, bytes: &[u8]) -> String {
        format!("{}  {filename}\n", hex32(&sha256(bytes)))
    }

    fn temp_home(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "hy-tui-{tag}-{nanos}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn parse_gnu_two_space_and_star() {
        let sums = "\
5111d2c2efd70daae34acb1488c34225cc9cf6522e82eb22dc7ef1401095ca5e  hy-linux-amd64
43ff483fb27cc94220d9e03cb00530e2c04d8f118258b0c6fb8ef10f3a39e0c7 *hy-linux-amd64-musl
";
        let map = parse_sha256sums(sums).unwrap();
        assert_eq!(
            hex32(map.get("hy-linux-amd64").unwrap()),
            "5111d2c2efd70daae34acb1488c34225cc9cf6522e82eb22dc7ef1401095ca5e"
        );
        assert_eq!(
            hex32(map.get("hy-linux-amd64-musl").unwrap()),
            "43ff483fb27cc94220d9e03cb00530e2c04d8f118258b0c6fb8ef10f3a39e0c7"
        );
        assert_ne!(
            map.get("hy-linux-amd64").unwrap(),
            map.get("hy-linux-amd64-musl").unwrap()
        );
    }

    #[test]
    fn parse_bsd_line() {
        let sums = "SHA256 (hy-darwin-arm64) = 5b14d93e13c49b6beae811085a045e5ee2469ab318d026c67dda3faee9ea21c6\n";
        let map = parse_sha256sums(sums).unwrap();
        assert!(map.contains_key("hy-darwin-arm64"));
    }

    #[test]
    fn verify_matching_hash_ok() {
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        assert!(verify_bytes(bytes, &sums, "hy-linux-amd64").is_ok());
    }

    #[test]
    fn verify_flipped_byte_err() {
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        let mut bad = bytes.to_vec();
        bad[0] ^= 0xff;
        let err = verify_bytes(&bad, &sums, "hy-linux-amd64").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("SHA256 mismatch") || msg.to_ascii_lowercase().contains("mismatch"),
            "{msg}"
        );
    }

    #[test]
    fn verify_wrong_name_err() {
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        assert!(verify_bytes(bytes, &sums, "hy-darwin-arm64").is_err());
        assert!(verify_bytes(bytes, &sums, "hy-linux-amd64-musl").is_err());
    }

    #[test]
    fn verify_never_ok_on_bad_hash() {
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        let mut bad = bytes.to_vec();
        for b in &mut bad {
            *b ^= 1;
        }
        assert!(verify_bytes(&bad, &sums, "hy-linux-amd64").is_err());
        assert!(install_verified(
            &temp_home("bad-hash-verify"),
            "hy-linux-amd64",
            &bad,
            &sums
        )
        .is_err());
    }

    #[test]
    fn install_temp_home_mode_0755_contents() {
        let home = temp_home("ok-install");
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        let path = install_verified(&home, "hy-linux-amd64", bytes, &sums).unwrap();
        assert_eq!(path, default_hy_bin(&home));
        assert!(path.is_file());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "hy must be 0755, got {mode:o}");
        let bin_mode = fs::metadata(home.join(".hy").join("bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(bin_mode, 0o700, ".hy/bin must be 0700 when created");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn empty_bin_dir_created_from_scratch() {
        let home = temp_home("empty-bin");
        assert!(!home.join(".hy").exists());
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-darwin-arm64", bytes);
        let path = install_verified(&home, "hy-darwin-arm64", bytes, &sums).unwrap();
        assert!(path.starts_with(&home));
        assert!(home.join(".hy").join("bin").is_dir());
        assert!(path.is_file());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn bad_hash_does_not_write_dest() {
        let home = temp_home("no-write");
        let dest = default_hy_bin(&home);
        assert!(!dest.exists());
        let bytes = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", bytes);
        let mut bad = bytes.to_vec();
        bad[0] ^= 1;
        let err = install_verified(&home, "hy-linux-amd64", &bad, &sums).unwrap_err();
        assert!(
            err.to_string().contains("SHA256 mismatch"),
            "{}",
            err
        );
        assert!(
            !dest.exists(),
            "bad hash must not create {}",
            dest.display()
        );
        // leftover temp must not become dest
        let bin = home.join(".hy").join("bin");
        if bin.is_dir() {
            for ent in fs::read_dir(&bin).unwrap() {
                let name = ent.unwrap().file_name();
                assert_ne!(name, "hy", "must not leave dest named hy");
            }
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn bad_hash_leaves_previous_good_file() {
        let home = temp_home("keep-good");
        let good = fixture_bytes();
        let sums = gnu_sums_for("hy-linux-amd64", good);
        let dest = install_verified(&home, "hy-linux-amd64", good, &sums).unwrap();
        let mut bad = good.to_vec();
        bad[3] ^= 0x5a;
        assert!(install_verified(&home, "hy-linux-amd64", &bad, &sums).is_err());
        assert_eq!(fs::read(&dest).unwrap(), good);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn prefer_gnu_sums_line_not_musl() {
        let gnu = b"gnu-bytes";
        let musl = b"musl-bytes";
        let sums = format!(
            "{}  hy-linux-amd64\n{}  hy-linux-amd64-musl\n",
            hex32(&sha256(gnu)),
            hex32(&sha256(musl))
        );
        assert!(verify_bytes(gnu, &sums, "hy-linux-amd64").is_ok());
        assert!(verify_bytes(musl, &sums, "hy-linux-amd64").is_err());
        assert!(verify_bytes(musl, &sums, "hy-linux-amd64-musl").is_ok());
    }

    /// Live GitHub fetch. Skips cleanly when the network/API is unavailable.
    #[test]
    fn live_fetch_latest_linux_amd64() {
        if !github_reachable() {
            eprintln!("skip live fetch: GitHub releases/latest not reachable");
            return;
        }
        let home = temp_home("live");
        let result = match install_latest_asset(&home, "hy-linux-amd64") {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skip live fetch: {e}");
                let _ = fs::remove_dir_all(&home);
                return;
            }
        };
        let dest = default_hy_bin(&home);
        assert_eq!(result.path, dest);
        assert!(dest.is_file(), "{}", dest.display());
        let mode = fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);

        let output = std::process::Command::new(&dest)
            .arg("version")
            .output()
            .expect("run hy version");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !text.trim().is_empty(),
            "hy version was empty (status {:?})",
            output.status
        );
        let _ = fs::remove_dir_all(&home);
    }
}
