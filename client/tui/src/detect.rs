//! Map OS / CPU to a wusir27/hy GitHub Release asset name (v0.0.2 pattern).
//!
//! Linux amd64 prefers the gnu asset (`hy-linux-amd64`) over `hy-linux-amd64-musl`.
//! Windows is mapped so a later fetch can look for an asset; TUN Start stays U4.

use anyhow::{bail, Result};

/// Asset filename for this OS/arch, using GitHub Release names (`hy-<os>-<arch>`).
///
/// `os` accepts `linux` / `macos` / `darwin` / `windows`.
/// `arch` accepts rustc names (`x86_64`, `aarch64`, `arm`, `x86`) and aliases.
pub fn asset_name(os: &str, arch: &str) -> Result<&'static str> {
    let os = normalize_os(os);
    let arch = arch.trim().to_ascii_lowercase();
    match (os.as_str(), arch.as_str()) {
        ("macos", "aarch64" | "arm64") => Ok("hy-darwin-arm64"),
        ("macos", "x86_64" | "amd64") => Ok("hy-darwin-amd64"),
        ("linux", "x86_64" | "amd64") => Ok("hy-linux-amd64"),
        ("linux", "aarch64" | "arm64") => Ok("hy-linux-arm64"),
        ("linux", "arm" | "armv7" | "armv7l" | "armv6" | "armv6l" | "armv5" | "armv5l") => {
            Ok("hy-linux-armv7")
        }
        ("linux", "x86" | "i686" | "i386" | "386") => Ok("hy-linux-386"),
        ("windows", "x86_64" | "amd64") => Ok("hy-windows-amd64"),
        ("windows", "aarch64" | "arm64") => Ok("hy-windows-arm64"),
        ("windows", "x86" | "i686" | "i386" | "386") => Ok("hy-windows-386"),
        _ => bail!("unsupported OS/arch {os}/{arch}"),
    }
}

fn normalize_os(os: &str) -> String {
    match os.trim().to_ascii_lowercase().as_str() {
        "darwin" => "macos".to_string(),
        other => other.to_string(),
    }
}

/// Asset name for the machine compiling/running this binary.
pub fn host_asset_name() -> Result<&'static str> {
    asset_name(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_x86_64_is_gnu_amd64_not_musl() {
        assert_eq!(asset_name("linux", "x86_64").unwrap(), "hy-linux-amd64");
        assert_eq!(asset_name("linux", "amd64").unwrap(), "hy-linux-amd64");
        assert_ne!(asset_name("linux", "x86_64").unwrap(), "hy-linux-amd64-musl");
    }

    #[test]
    fn darwin_aarch64_and_x86_64() {
        assert_eq!(asset_name("darwin", "aarch64").unwrap(), "hy-darwin-arm64");
        assert_eq!(asset_name("macos", "aarch64").unwrap(), "hy-darwin-arm64");
        assert_eq!(asset_name("macos", "arm64").unwrap(), "hy-darwin-arm64");
        assert_eq!(asset_name("macos", "x86_64").unwrap(), "hy-darwin-amd64");
        assert_eq!(asset_name("darwin", "amd64").unwrap(), "hy-darwin-amd64");
    }

    #[test]
    fn linux_arm_and_386() {
        assert_eq!(asset_name("linux", "aarch64").unwrap(), "hy-linux-arm64");
        assert_eq!(asset_name("linux", "arm64").unwrap(), "hy-linux-arm64");
        assert_eq!(asset_name("linux", "arm").unwrap(), "hy-linux-armv7");
        assert_eq!(asset_name("linux", "armv7").unwrap(), "hy-linux-armv7");
        assert_eq!(asset_name("linux", "armv7l").unwrap(), "hy-linux-armv7");
        assert_eq!(asset_name("linux", "x86").unwrap(), "hy-linux-386");
        assert_eq!(asset_name("linux", "i686").unwrap(), "hy-linux-386");
        assert_eq!(asset_name("linux", "i386").unwrap(), "hy-linux-386");
        assert_eq!(asset_name("linux", "386").unwrap(), "hy-linux-386");
    }

    #[test]
    fn windows_maps_hy_asset_name() {
        assert_eq!(asset_name("windows", "x86_64").unwrap(), "hy-windows-amd64");
        assert_eq!(asset_name("windows", "aarch64").unwrap(), "hy-windows-arm64");
    }

    #[test]
    fn unsupported_triple_errors() {
        assert!(asset_name("freebsd", "x86_64").is_err());
        assert!(asset_name("linux", "riscv64").is_err());
        assert!(asset_name("linux", "s390x").is_err());
        assert!(asset_name("solaris", "sparc").is_err());
    }

    #[test]
    fn host_asset_name_is_known_on_this_ci() {
        let name = host_asset_name().expect("this CI OS/arch should map to a hy asset");
        assert!(
            name.starts_with("hy-"),
            "unexpected host asset {name}"
        );
    }
}
