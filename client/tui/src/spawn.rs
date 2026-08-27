//! Build `hy client` argv for Start (U3). No sudo, no SIGINT Stop, no exec in the UI.
//!
//! Command: `<hy_bin> client -c <abs client.yaml> [--route <abs route>]`
//! Absolute paths only. Route mode off omits `--route`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::config_gen::{FormState, RouteMode};
use crate::fetch_hy;
use crate::fetch_route::{self, FetchResult};

/// Prepared Start: argv to show (and later exec in U4) plus optional RULE-SET warning.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PreparedStart {
    pub argv: Vec<String>,
    pub ruleset_warning: bool,
    pub route_path: Option<PathBuf>,
}

pub fn client_yaml_path(home: &Path) -> PathBuf {
    home.join(".hy").join("client.yaml")
}

pub fn hy_bin_path(home: &Path, form: &FormState) -> PathBuf {
    let custom = form.hy_path.trim();
    if custom.is_empty() {
        fetch_hy::default_hy_bin(home)
    } else {
        PathBuf::from(custom)
    }
}

/// Make a path absolute without requiring it to exist (cwd join).
pub fn abs_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::path::absolute(p) {
        Ok(abs) => abs,
        Err(_) => std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf()),
    }
}

/// `hy_bin client -c <abs yaml> [--route <abs route>]`.
pub fn start_argv(hy_bin: &Path, client_yaml: &Path, route: Option<&Path>) -> Vec<String> {
    let mut argv = vec![
        abs_path(hy_bin).to_string_lossy().into_owned(),
        "client".into(),
        "-c".into(),
        abs_path(client_yaml).to_string_lossy().into_owned(),
    ];
    if let Some(route) = route {
        argv.push("--route".into());
        argv.push(abs_path(route).to_string_lossy().into_owned());
    }
    argv
}

pub fn format_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.chars().any(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Download (url) / require local file / skip (off), then build argv. Does not exec hy.
pub fn prepare_start(home: &Path, form: &FormState) -> Result<PreparedStart> {
    let hy = abs_path(&hy_bin_path(home, form));
    let yaml = abs_path(&client_yaml_path(home));

    match form.route_mode {
        RouteMode::Off => Ok(PreparedStart {
            argv: start_argv(&hy, &yaml, None),
            ruleset_warning: false,
            route_path: None,
        }),
        RouteMode::Local => {
            let raw = form.route_local_path.trim();
            if raw.is_empty() {
                bail!("local route path is empty");
            }
            let path = abs_path(Path::new(raw));
            if !path.is_file() {
                bail!("local route file not found: {}", path.display());
            }
            Ok(PreparedStart {
                argv: start_argv(&hy, &yaml, Some(&path)),
                ruleset_warning: false,
                route_path: Some(path),
            })
        }
        RouteMode::Url => {
            let url = form.route_url.trim();
            if url.is_empty() {
                bail!("route URL is empty");
            }
            let FetchResult {
                path,
                ruleset_warning,
            } = fetch_route::fetch_to_home(home, url)?;
            let path = abs_path(&path);
            Ok(PreparedStart {
                argv: start_argv(&hy, &yaml, Some(&path)),
                ruleset_warning,
                route_path: Some(path),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch_route::{self, SR_CNIP_FIXTURE};

    fn temp_home(tag: &str) -> PathBuf {
        fetch_route::temp_home(tag)
    }

    fn has_route(argv: &[String]) -> bool {
        argv.iter().any(|a| a == "--route")
    }

    fn route_arg(argv: &[String]) -> &str {
        let i = argv
            .iter()
            .position(|a| a == "--route")
            .expect("expected --route");
        argv.get(i + 1).expect("--route value")
    }

    #[test]
    fn start_argv_off_has_no_route() {
        let home = PathBuf::from("/tmp/hy-tui-argv-off");
        let hy = fetch_hy::default_hy_bin(&home);
        let yaml = client_yaml_path(&home);
        let argv = start_argv(&hy, &yaml, None);
        assert_eq!(argv[1], "client");
        assert_eq!(argv[2], "-c");
        assert!(!has_route(&argv), "off must not pass --route: {argv:?}");
        assert!(Path::new(&argv[0]).is_absolute(), "{}", argv[0]);
        assert!(Path::new(&argv[3]).is_absolute(), "{}", argv[3]);
    }

    #[test]
    fn prepare_start_off_does_not_write_route_conf() {
        let home = temp_home("argv-off");
        let form = FormState::default();
        assert_eq!(form.route_mode, RouteMode::Off);
        let prepared = prepare_start(&home, &form).unwrap();
        assert!(!has_route(&prepared.argv), "{:?}", prepared.argv);
        assert!(!fetch_route::default_route_conf(&home).exists());
        assert!(!prepared.ruleset_warning);
        let cmd = format_argv(&prepared.argv);
        assert!(!cmd.contains("--route"), "{cmd}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn start_argv_url_after_download_has_abs_route() {
        let home = temp_home("argv-url");
        let fetched = fetch_route::install_bytes(&home, SR_CNIP_FIXTURE.as_bytes()).unwrap();
        let hy = fetch_hy::default_hy_bin(&home);
        let yaml = client_yaml_path(&home);
        let argv = start_argv(&hy, &yaml, Some(&fetched.path));
        assert!(has_route(&argv), "{argv:?}");
        let route = Path::new(route_arg(&argv));
        assert!(route.is_absolute(), "{route:?}");
        assert_eq!(route, fetched.path);
        assert_eq!(route.file_name().and_then(|n| n.to_str()), Some("route.conf"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn start_argv_local_existing_file_has_abs_route() {
        let home = temp_home("argv-local");
        let local = home.join("custom.conf");
        std::fs::write(&local, SR_CNIP_FIXTURE.as_bytes()).unwrap();

        let mut form = FormState::default();
        form.route_mode = RouteMode::Local;
        form.route_local_path = local.to_string_lossy().into_owned();
        let prepared = prepare_start(&home, &form).unwrap();
        assert!(has_route(&prepared.argv), "{:?}", prepared.argv);
        let route = Path::new(route_arg(&prepared.argv));
        assert!(route.is_absolute(), "{route:?}");
        assert_eq!(route, &local);
        assert!(!fetch_route::default_route_conf(&home).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn prepare_start_local_missing_errors() {
        let home = temp_home("argv-missing");
        let mut form = FormState::default();
        form.route_mode = RouteMode::Local;
        form.route_local_path = home.join("no-such.conf").to_string_lossy().into_owned();
        let err = prepare_start(&home, &form).unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("not found")
                || err.to_string().contains("no-such"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
