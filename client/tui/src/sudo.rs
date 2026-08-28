//! System sudo wrapper to launch `hy client` (U4). TUI stays a normal user.
//!
//! Never setuid. Never SIGKILL on the regular Stop path. Password is stdin-only
//! (`sudo -S`), then zeroized. Not written to disk, logs, tui.json, or status.

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use zeroize::{Zeroize, Zeroizing};

use crate::config_gen;
use crate::logbuf::{self, LogTap};
use crate::spawn::{self, PreparedStart};

pub const PRESERVE_ENV: &str = "HYSTERIA_LOG_LEVEL,HYSTERIA_LOG_FORMAT";
pub const LOG_LEVEL: &str = "info";
pub const LOG_FORMAT: &str = "console";
pub const PASSWORD_PROMPT: &str = "系统密码，给 sudo 用一次";
pub const NO_TTY_MSG: &str = "在 Terminal 里跑 hy-tui";
pub const MAX_PASSWORD_FAILS: u8 = 3;
pub const STOP_WAIT: Duration = Duration::from_secs(8);
const PID_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privilege {
    Root,
    SudoCached,
    NeedPassword,
}

impl Privilege {
    pub fn via_sudo(self) -> bool {
        !matches!(self, Self::Root)
    }

    pub fn needs_password(self) -> bool {
        matches!(self, Self::NeedPassword)
    }
}

pub fn classify_privilege(euid: u32, sudo_n_true: bool) -> Privilege {
    if euid == 0 {
        Privilege::Root
    } else if sudo_n_true {
        Privilege::SudoCached
    } else {
        Privilege::NeedPassword
    }
}

pub fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn sudo_n_true() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn probe_privilege() -> Privilege {
    classify_privilege(effective_uid(), sudo_n_true())
}

pub fn has_tty() -> bool {
    io::stdin().is_terminal()
}

/// True when the YAML mapping has a top-level `tun:` block.
pub fn yaml_has_tun(yaml: &str) -> bool {
    if let Ok(serde_yaml::Value::Mapping(map)) = serde_yaml::from_str(yaml) {
        return map.contains_key(serde_yaml::Value::String("tun".into()));
    }
    yaml.lines().any(|l| {
        let t = l.trim_start();
        t == "tun:" || t.starts_with("tun:")
    })
}

pub fn require_tun_for_start(yaml: &str) -> Result<()> {
    if !yaml_has_tun(yaml) {
        bail!("yaml has no tun: block; Start refuses sudo");
    }
    Ok(())
}

fn sh_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.bytes().all(|b| {
        matches!(
            b,
            b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'/'
                | b'.'
                | b'_'
                | b'-'
                | b'='
                | b':'
                | b'+'
                | b'@'
                | b'%'
                | b','
        )
    }) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// `echo $$; exec <abs hy> client -c <abs yaml> [--route <abs>]`
pub fn inner_exec_script(prepared: &PreparedStart) -> String {
    let mut out = String::from("echo $$; exec");
    for a in &prepared.argv {
        out.push(' ');
        out.push_str(&sh_quote(a));
    }
    out
}

pub fn wrapper_argv(prepared: &PreparedStart, privilege: Privilege) -> Vec<String> {
    let script = inner_exec_script(prepared);
    match privilege {
        Privilege::Root => vec!["/bin/sh".into(), "-c".into(), script],
        Privilege::SudoCached => vec![
            "sudo".into(),
            "-n".into(),
            format!("--preserve-env={PRESERVE_ENV}"),
            "/bin/sh".into(),
            "-c".into(),
            script,
        ],
        Privilege::NeedPassword => vec![
            "sudo".into(),
            "-S".into(),
            "-p".into(),
            String::new(),
            format!("--preserve-env={PRESERVE_ENV}"),
            "/bin/sh".into(),
            "-c".into(),
            script,
        ],
    }
}

/// `sudo -n kill -INT <pid>` (or `kill -INT` when we did not use sudo / already root).
/// Never SIGKILL.
pub fn stop_argv(pid: u32, via_sudo: bool) -> Vec<String> {
    kill_argv(pid, via_sudo, "-INT")
}

/// Timeout fallback after SIGINT. Never SIGKILL.
pub fn term_argv(pid: u32, via_sudo: bool) -> Vec<String> {
    kill_argv(pid, via_sudo, "-TERM")
}

fn kill_argv(pid: u32, via_sudo: bool, sig: &str) -> Vec<String> {
    if via_sudo {
        vec![
            "sudo".into(),
            "-n".into(),
            "kill".into(),
            sig.into(),
            pid.to_string(),
        ]
    } else {
        vec!["kill".into(), sig.into(), pid.to_string()]
    }
}

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub via_sudo: bool,
    pub needs_password: bool,
    pub ruleset_warning: bool,
}

pub fn plan_launch(
    home: &Path,
    yaml: &str,
    prepared: &PreparedStart,
    privilege: Privilege,
) -> Result<LaunchPlan> {
    require_tun_for_start(yaml)?;
    Ok(LaunchPlan {
        argv: wrapper_argv(prepared, privilege),
        cwd: home.join(".hy"),
        via_sudo: privilege.via_sudo(),
        needs_password: privilege.needs_password(),
        ruleset_warning: prepared.ruleset_warning,
    })
}

/// Save yaml, prepare `--route` argv, refuse Start without `tun:`, wrap with sudo.
pub fn prepare_launch(home: &Path, form: &config_gen::FormState) -> Result<LaunchPlan> {
    let yaml = config_gen::to_yaml(form);
    let prepared = spawn::prepare_start(home, form)?;
    plan_launch(home, &yaml, &prepared, probe_privilege())
}

#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    pub path_prepend: Option<PathBuf>,
    pub extra_env: Vec<(String, String)>,
}

pub struct HyProcess {
    pub hy_pid: u32,
    pub via_sudo: bool,
    child: Option<Child>,
    drains: Vec<thread::JoinHandle<()>>,
    path_prepend: Option<PathBuf>,
    extra_env: Vec<(String, String)>,
    finished: bool,
    log_tap: LogTap,
}

impl HyProcess {
    pub fn is_alive(&mut self) -> bool {
        if self.finished {
            return false;
        }
        let Some(child) = self.child.as_mut() else {
            self.finished = true;
            self.join_drains();
            return false;
        };
        match child.try_wait() {
            Ok(Some(_)) => {
                self.finished = true;
                self.join_drains();
                false
            }
            _ => true,
        }
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<bool> {
        let start = Instant::now();
        loop {
            let Some(child) = self.child.as_mut() else {
                self.finished = true;
                self.join_drains();
                return Ok(true);
            };
            if let Some(_status) = child.try_wait()? {
                self.finished = true;
                self.join_drains();
                return Ok(true);
            }
            if start.elapsed() >= timeout {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// SIGINT, wait, then SIGTERM. Never SIGKILL.
    pub fn stop(&mut self, timeout: Duration) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        let _ = run_argv(
            &stop_argv(self.hy_pid, self.via_sudo),
            self.path_prepend.as_deref(),
            &self.extra_env,
        );
        if self.wait_timeout(timeout)? {
            return Ok(());
        }
        let _ = run_argv(
            &term_argv(self.hy_pid, self.via_sudo),
            self.path_prepend.as_deref(),
            &self.extra_env,
        );
        if self.wait_timeout(timeout)? {
            return Ok(());
        }
        bail!("hy pid {} did not exit after SIGINT/SIGTERM", self.hy_pid)
    }

    fn join_drains(&mut self) {
        for h in self.drains.drain(..) {
            let _ = h.join();
        }
    }

    pub fn take_log_lines(&self) -> Vec<String> {
        self.log_tap.take()
    }
}

impl Drop for HyProcess {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = run_argv(
            &stop_argv(self.hy_pid, self.via_sudo),
            self.path_prepend.as_deref(),
            &self.extra_env,
        );
        let _ = self.wait_timeout(Duration::from_secs(1));
        if self.finished {
            return;
        }
        let _ = run_argv(
            &term_argv(self.hy_pid, self.via_sudo),
            self.path_prepend.as_deref(),
            &self.extra_env,
        );
        let _ = self.wait_timeout(Duration::from_secs(1));
        if !self.finished {
            // Do not SIGKILL. Detach so Drop does not block the TUI.
            if let Some(child) = self.child.take() {
                std::mem::forget(child);
            }
        }
    }
}

#[derive(Default)]
pub struct StartSession {
    pub process: Option<HyProcess>,
    pub password_failures: u8,
    log_tap: LogTap,
}

impl StartSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_child(&self) -> bool {
        self.process.is_some()
    }

    pub fn begin_start(&mut self) {
        self.password_failures = 0;
    }

    pub fn record_child(&mut self, proc: HyProcess) {
        self.log_tap = proc.log_tap.clone();
        self.process = Some(proc);
        self.password_failures = 0;
    }

    pub fn poll_logs(&self) -> Vec<String> {
        self.log_tap.take()
    }

    pub fn note_password_failure(&mut self) {
        self.process = None;
        self.password_failures = self.password_failures.saturating_add(1);
    }

    pub fn cancel(&mut self) {
        self.process = None;
    }

    pub fn password_locked_out(&self) -> bool {
        self.password_failures >= MAX_PASSWORD_FAILS
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut proc) = self.process.take() {
            proc.stop(STOP_WAIT)?;
        }
        Ok(())
    }
}

fn resolve_bin(name: &str, path_prepend: Option<&Path>) -> PathBuf {
    if let Some(dir) = path_prepend {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(name)
}

fn apply_path_and_env(
    cmd: &mut Command,
    path_prepend: Option<&Path>,
    extra_env: &[(String, String)],
) {
    if let Some(dir) = path_prepend {
        let old = std::env::var("PATH").unwrap_or_default();
        cmd.env("PATH", format!("{}:{old}", dir.display()));
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
}

fn run_argv(
    argv: &[String],
    path_prepend: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    if argv.is_empty() {
        bail!("empty argv");
    }
    let bin = resolve_bin(&argv[0], path_prepend);
    let mut cmd = Command::new(&bin);
    cmd.args(&argv[1..]);
    apply_path_and_env(&mut cmd, path_prepend, extra_env);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("run {}", argv.join(" ")))
}

fn drain_read<R: Read + Send + 'static>(mut r: R, tap: LogTap) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut partial = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => {
                    logbuf::flush_partial(&mut partial, |s| tap.push(s));
                    break;
                }
                Ok(n) => {
                    logbuf::feed_bytes(&mut partial, &buf[..n], |s| tap.push(s));
                }
            }
        }
    })
}

fn read_first_line(
    mut stdout: std::process::ChildStdout,
    timeout: Duration,
) -> Result<(String, std::process::ChildStdout)> {
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        let result = loop {
            match stdout.read(&mut byte) {
                Ok(0) => break Ok((line, stdout)),
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break Ok((line, stdout));
                    }
                    line.push(byte[0]);
                    if line.len() > 64 {
                        break Ok((line, stdout));
                    }
                }
                Err(e) => break Err((e, stdout)),
            }
        };
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok((line, stdout))) => {
            let _ = worker.join();
            let text = String::from_utf8_lossy(&line).trim().to_string();
            Ok((text, stdout))
        }
        Ok(Err((e, _stdout))) => {
            let _ = worker.join();
            Err(e).context("read hy pid line")
        }
        Err(_) => {
            bail!("timed out waiting for hy pid line");
        }
    }
}

fn sanitize_err(msg: &str, password: Option<&str>) -> String {
    let mut out: String = msg
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(240)
        .collect();
    if let Some(pw) = password {
        if !pw.is_empty() {
            out = out.replace(pw, "****");
        }
    }
    out
}

/// Spawn the sudo/sh wrapper. Password is written once to sudo stdin, then zeroized.
/// On failure the child is reaped and not returned.
pub fn spawn_hy(
    plan: &LaunchPlan,
    mut password: Option<Zeroizing<String>>,
    opts: SpawnOpts,
) -> Result<HyProcess> {
    if plan.argv.is_empty() {
        if let Some(ref mut pw) = password {
            Zeroize::zeroize(pw);
        }
        bail!("empty launch argv");
    }
    std::fs::create_dir_all(&plan.cwd).with_context(|| format!("create {}", plan.cwd.display()))?;

    let bin = resolve_bin(&plan.argv[0], opts.path_prepend.as_deref());
    let mut cmd = Command::new(&bin);
    cmd.args(&plan.argv[1..]);
    cmd.current_dir(&plan.cwd);
    cmd.env("HYSTERIA_LOG_LEVEL", LOG_LEVEL);
    cmd.env("HYSTERIA_LOG_FORMAT", LOG_FORMAT);
    apply_path_and_env(&mut cmd, opts.path_prepend.as_deref(), &opts.extra_env);

    let needs_stdin = plan.argv.iter().any(|a| a == "-S");
    if needs_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            if let Some(ref mut pw) = password {
                Zeroize::zeroize(pw);
            }
            return Err(e).context("spawn sudo/hy");
        }
    };

    if needs_stdin {
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.wait();
            if let Some(ref mut pw) = password {
                Zeroize::zeroize(pw);
            }
            bail!("sudo stdin missing");
        };
        if let Some(ref pw) = password {
            let _ = stdin.write_all(pw.as_bytes());
            if !pw.ends_with('\n') {
                let _ = stdin.write_all(b"\n");
            }
            let _ = stdin.flush();
        } else {
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
        drop(stdin);
    }
    if let Some(ref mut pw) = password {
        Zeroize::zeroize(pw);
    }
    drop(password.take());

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.wait();
            bail!("hy stdout missing");
        }
    };

    let (pid_line, stdout_rest) = match read_first_line(stdout, PID_WAIT) {
        Ok(v) => v,
        Err(e) => {
            terminate_wrapper(&mut child);
            let err = fail_spawn(&mut child, &e.to_string(), None);
            abandon_if_running(child);
            return Err(err);
        }
    };

    let hy_pid: u32 = match pid_line.parse() {
        Ok(p) if p > 0 => p,
        _ => {
            terminate_wrapper(&mut child);
            let err = fail_spawn(
                &mut child,
                &format!("invalid hy pid line: {pid_line:?}"),
                None,
            );
            abandon_if_running(child);
            return Err(err);
        }
    };

    let tap = LogTap::new();
    let mut drains = vec![drain_read(stdout_rest, tap.clone())];
    if let Some(stderr) = child.stderr.take() {
        drains.push(drain_read(stderr, tap.clone()));
    }

    let mut proc = HyProcess {
        hy_pid,
        via_sudo: plan.via_sudo,
        child: Some(child),
        drains,
        path_prepend: opts.path_prepend,
        extra_env: opts.extra_env,
        finished: false,
        log_tap: tap,
    };
    thread::sleep(Duration::from_millis(30));
    if !proc.is_alive() {
        bail!("hy pid {hy_pid} exited immediately");
    }
    Ok(proc)
}

fn terminate_wrapper(child: &mut Child) {
    match child.try_wait() {
        Ok(None) => unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        },
        _ => {}
    }
}

fn abandon_if_running(mut child: Child) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        _ => {
            std::mem::forget(child);
        }
    }
}

fn fail_spawn(child: &mut Child, why: &str, password: Option<&str>) -> anyhow::Error {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        match child.try_wait() {
            Ok(Some(_)) => break,
            _ => thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut tail = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = [0u8; 1024];
        if let Ok(n) = stderr.read(&mut buf) {
            tail = String::from_utf8_lossy(&buf[..n]).into_owned();
        }
    }
    let msg = if tail.trim().is_empty() {
        why.to_string()
    } else {
        format!("{why}: {}", tail.trim())
    };
    anyhow::anyhow!("{}", sanitize_err(&msg, password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_gen::{FormState, RouteMode};
    use crate::fetch_hy;
    use crate::fetch_route::{self, SR_CNIP_FIXTURE};
    use crate::spawn::{client_yaml_path, prepare_start, start_argv};
    use std::os::unix::fs::PermissionsExt;

    fn temp_home(tag: &str) -> PathBuf {
        fetch_route::temp_home(&format!("sudo-{tag}"))
    }

    fn joined(argv: &[String]) -> String {
        argv.join(" ")
    }

    fn inner(argv: &[String]) -> &str {
        argv.last().map(|s| s.as_str()).unwrap_or("")
    }

    fn has_sigkill(argv: &[String]) -> bool {
        argv.iter().any(|a| {
            let u = a.to_ascii_uppercase();
            u == "-KILL" || u == "-9" || u == "SIGKILL" || u.contains("SIGKILL")
        })
    }

    fn yaml_with_tun() -> String {
        config_gen::to_yaml(&FormState::default())
    }

    const FAKE_SUDO: &str = r#"#!/bin/sh
need_s=0
for a in "$@"; do
  if [ "$a" = "-S" ]; then
    need_s=1
  fi
done
if [ "$need_s" = 1 ]; then
  IFS= read -r _pw || true
  unset _pw
  if [ "${HY_TUI_FAKE_BAD_PASS:-0}" = 1 ]; then
    echo "sudo: 3 incorrect password attempts" >&2
    exit 1
  fi
fi
if [ "$1" = "-n" ] && [ "$2" = "true" ] && [ $# -eq 2 ]; then
  if [ "${HY_TUI_FAKE_N_TRUE:-0}" = 1 ]; then
    exit 0
  fi
  exit 1
fi
while [ $# -gt 0 ]; do
  case "$1" in
    -n|-S)
      shift
      ;;
    -p)
      shift
      if [ $# -gt 0 ]; then shift; fi
      ;;
    --preserve-env=*)
      shift
      ;;
    --preserve-env)
      shift
      if [ $# -gt 0 ]; then shift; fi
      ;;
    true)
      shift
      ;;
    kill)
      shift
      if [ -x /bin/kill ]; then
        exec /bin/kill "$@"
      fi
      exec /usr/bin/kill "$@"
      ;;
    /bin/sh)
      shift
      exec /bin/sh "$@"
      ;;
    *)
      shift
      ;;
  esac
done
exit 1
"#;

    const FAKE_HY: &str = r#"#!/bin/sh
trap 'exit 0' INT TERM
while true; do
  sleep 1
done
"#;

    fn write_exec(path: &Path, body: &str) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(path, body).unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    fn install_fake_hy(home: &Path) -> PathBuf {
        let hy = fetch_hy::default_hy_bin(home);
        write_exec(&hy, FAKE_HY);
        hy
    }

    fn install_fake_sudo(home: &Path) -> PathBuf {
        let dir = home.join("fake-bin");
        write_exec(&dir.join("sudo"), FAKE_SUDO);
        dir
    }

    fn default_prepared(home: &Path) -> PreparedStart {
        let hy = fetch_hy::default_hy_bin(home);
        let yaml = client_yaml_path(home);
        PreparedStart {
            argv: start_argv(&hy, &yaml, None),
            ruleset_warning: false,
            route_path: None,
        }
    }

    #[test]
    fn classify_root_skips_sudo() {
        assert_eq!(classify_privilege(0, false), Privilege::Root);
        assert_eq!(classify_privilege(0, true), Privilege::Root);
        assert!(!classify_privilege(0, false).via_sudo());
        assert!(!classify_privilege(0, false).needs_password());
    }

    #[test]
    fn classify_cached_uses_sudo_n() {
        let p = classify_privilege(1000, true);
        assert_eq!(p, Privilege::SudoCached);
        assert!(p.via_sudo());
        assert!(!p.needs_password());
    }

    #[test]
    fn classify_uncached_needs_password() {
        let p = classify_privilege(1000, false);
        assert_eq!(p, Privilege::NeedPassword);
        assert!(p.via_sudo());
        assert!(p.needs_password());
    }

    #[test]
    fn sudo_n_wrapper_contains_required_pieces_and_route() {
        let home = temp_home("wrap-route");
        let fetched = fetch_route::install_bytes(&home, SR_CNIP_FIXTURE.as_bytes()).unwrap();
        let hy = fetch_hy::default_hy_bin(&home);
        let yaml = client_yaml_path(&home);
        let prepared = PreparedStart {
            argv: start_argv(&hy, &yaml, Some(&fetched.path)),
            ruleset_warning: false,
            route_path: Some(fetched.path.clone()),
        };
        let argv = wrapper_argv(&prepared, Privilege::SudoCached);
        let blob = joined(&argv);
        assert!(argv.contains(&"sudo".to_string()), "{argv:?}");
        assert!(argv.contains(&"-n".to_string()), "{argv:?}");
        assert!(
            argv.iter().any(|a| a.contains("--preserve-env=")
                && a.contains("HYSTERIA_LOG_LEVEL")
                && a.contains("HYSTERIA_LOG_FORMAT")),
            "{argv:?}"
        );
        let script = inner(&argv);
        assert!(script.contains("echo $$; exec"), "{script}");
        assert!(Path::new(&prepared.argv[0]).is_absolute());
        assert!(script.contains(&prepared.argv[0]), "{script}");
        assert!(script.contains("client"), "{script}");
        assert!(script.contains("-c"), "{script}");
        assert!(script.contains("--route"), "{script}");
        assert!(
            script.contains(&fetched.path.to_string_lossy().into_owned()),
            "{script}"
        );
        assert!(blob.contains("sudo") && blob.contains("-n"), "{blob}");
        assert!(!has_sigkill(&argv), "{argv:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn sudo_s_wrapper_uses_dash_s_and_empty_prompt() {
        let home = PathBuf::from("/tmp/hy-tui-sudo-s");
        let prepared = default_prepared(&home);
        let argv = wrapper_argv(&prepared, Privilege::NeedPassword);
        assert!(argv.contains(&"sudo".to_string()), "{argv:?}");
        assert!(argv.contains(&"-S".to_string()), "{argv:?}");
        assert!(argv.contains(&"-p".to_string()), "{argv:?}");
        let p = argv.iter().position(|a| a == "-p").unwrap();
        assert_eq!(argv.get(p + 1).map(String::as_str), Some(""));
        assert!(inner(&argv).contains("echo $$; exec"));
        assert!(
            !argv.contains(&"-n".to_string()) || argv.iter().filter(|a| *a == "-n").count() == 0
        );
        assert!(!argv.contains(&"-n".to_string()), "{argv:?}");
    }

    #[test]
    fn wrapper_off_has_no_route_in_inner_exec() {
        let home = PathBuf::from("/tmp/hy-tui-wrap-off");
        let prepared = default_prepared(&home);
        assert!(!prepared.argv.iter().any(|a| a == "--route"));
        for privs in [
            Privilege::SudoCached,
            Privilege::NeedPassword,
            Privilege::Root,
        ] {
            let argv = wrapper_argv(&prepared, privs);
            let script = inner(&argv);
            assert!(script.contains("echo $$; exec"), "{script}");
            assert!(
                !script.contains("--route"),
                "off must not pass --route in inner exec: {script}"
            );
            assert!(!argv.iter().any(|a| a.contains("--route")), "{argv:?}");
        }
    }

    #[test]
    fn no_tun_yaml_refuses_sudo_no_spawn() {
        let home = PathBuf::from("/tmp/hy-tui-no-tun");
        let prepared = default_prepared(&home);
        let yaml =
            "server: 127.0.0.1:443\nauth: secret\ntls:\n  sni: localhost\n  insecure: true\n";
        assert!(!yaml_has_tun(yaml));
        let err = plan_launch(&home, yaml, &prepared, Privilege::SudoCached).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("tun") || msg.to_ascii_lowercase().contains("sudo"),
            "{msg}"
        );
        assert!(yaml_has_tun(&yaml_with_tun()));
    }

    #[test]
    fn default_form_yaml_has_tun_so_start_uses_sudo() {
        let home = PathBuf::from("/tmp/hy-tui-default-tun");
        let form = FormState::default();
        let yaml = config_gen::to_yaml(&form);
        assert!(yaml_has_tun(&yaml), "{yaml}");
        let prepared = default_prepared(&home);
        let plan = plan_launch(&home, &yaml, &prepared, Privilege::SudoCached).unwrap();
        assert!(plan.argv.contains(&"sudo".to_string()));
        assert!(plan.argv.contains(&"-n".to_string()));
        assert!(plan.via_sudo);
    }

    #[test]
    fn stop_argv_is_sudo_n_kill_int_and_has_no_sigkill() {
        let a = stop_argv(4242, true);
        assert_eq!(
            a,
            vec!["sudo", "-n", "kill", "-INT", "4242"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert!(!has_sigkill(&a), "{a:?}");
        let b = stop_argv(7, false);
        assert_eq!(
            b,
            vec!["kill", "-INT", "7"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert!(!has_sigkill(&b), "{b:?}");
        let t = term_argv(9, true);
        assert_eq!(
            t,
            vec!["sudo", "-n", "kill", "-TERM", "9"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert!(!has_sigkill(&t), "{t:?}");
        assert!(!t.iter().any(|s| s == "-INT"));
    }

    #[test]
    fn password_not_in_written_files_after_mocked_s_start() {
        let home = temp_home("pass-files");
        install_fake_hy(&home);
        let fake_bin = install_fake_sudo(&home);
        let hy_dir = home.join(".hy");
        std::fs::create_dir_all(&hy_dir).unwrap();

        let mut form = FormState::default();
        form.server = "127.0.0.1:443".into();
        form.auth = "yaml-auth-secret".into();
        let yaml_path = client_yaml_path(&home);
        std::fs::write(&yaml_path, config_gen::to_yaml(&form)).unwrap();
        std::fs::write(hy_dir.join("tui.json"), "{\"advanced\":false}\n").unwrap();
        std::fs::write(hy_dir.join("route.conf"), "FINAL,PROXY\n").unwrap();

        let prepared = prepare_start(&home, &form).unwrap();
        let yaml = config_gen::to_yaml(&form);
        let plan = plan_launch(&home, &yaml, &prepared, Privilege::NeedPassword).unwrap();
        assert!(plan.argv.contains(&"-S".to_string()), "{:?}", plan.argv);

        let secret = "uniq-sudo-pw-U4-test-9f3a";
        assert!(!yaml.contains(secret));
        let mut proc = spawn_hy(
            &plan,
            Some(Zeroizing::new(secret.to_string())),
            SpawnOpts {
                path_prepend: Some(fake_bin.clone()),
                extra_env: vec![],
            },
        )
        .expect("mocked -S start");

        for name in ["client.yaml", "tui.json", "route.conf"] {
            let p = hy_dir.join(name);
            let body = std::fs::read_to_string(&p).unwrap_or_default();
            assert!(
                !body.contains(secret),
                "password leaked into {}: {body}",
                p.display()
            );
        }
        // Walk ~/.hy so a future writer cannot hide the password.
        if let Ok(entries) = std::fs::read_dir(&hy_dir) {
            for ent in entries.flatten() {
                let path = ent.path();
                if path.is_file() {
                    if let Ok(body) = std::fs::read_to_string(&path) {
                        assert!(
                            !body.contains(secret),
                            "password leaked into {}",
                            path.display()
                        );
                    }
                }
            }
        }

        proc.stop(Duration::from_secs(3)).ok();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn three_bad_passwords_and_cancel_record_no_child() {
        let home = temp_home("bad-pass");
        install_fake_hy(&home);
        let fake_bin = install_fake_sudo(&home);
        std::fs::create_dir_all(home.join(".hy")).unwrap();
        let form = FormState::default();
        std::fs::write(client_yaml_path(&home), config_gen::to_yaml(&form)).unwrap();

        let prepared = prepare_start(&home, &form).unwrap();
        let plan = plan_launch(
            &home,
            &config_gen::to_yaml(&form),
            &prepared,
            Privilege::NeedPassword,
        )
        .unwrap();

        let mut session = StartSession::new();
        session.begin_start();
        for _ in 0..MAX_PASSWORD_FAILS {
            let result = spawn_hy(
                &plan,
                Some(Zeroizing::new("wrong-password".into())),
                SpawnOpts {
                    path_prepend: Some(fake_bin.clone()),
                    extra_env: vec![("HY_TUI_FAKE_BAD_PASS".into(), "1".into())],
                },
            );
            assert!(result.is_err(), "bad password must not spawn hy");
            session.note_password_failure();
            assert!(!session.has_child());
        }
        assert!(session.password_locked_out());
        assert!(!session.has_child());

        let mut cancelled = StartSession::new();
        cancelled.begin_start();
        cancelled.cancel();
        assert!(!cancelled.has_child());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn prepare_start_off_still_omits_route_through_wrapper() {
        let home = temp_home("prep-off");
        let mut form = FormState::default();
        form.route_mode = RouteMode::Off;
        let prepared = prepare_start(&home, &form).unwrap();
        let plan = plan_launch(
            &home,
            &config_gen::to_yaml(&form),
            &prepared,
            Privilege::SudoCached,
        )
        .unwrap();
        assert!(!inner(&plan.argv).contains("--route"), "{:?}", plan.argv);
        let _ = std::fs::remove_dir_all(&home);
    }

    const FAKE_HY_LOG: &str = r#"#!/bin/sh
echo "hello-out tun up"
echo "authenticated" >&2
trap 'exit 0' INT TERM
while true; do
  sleep 1
done
"#;

    #[test]
    fn remaining_stdout_stderr_reach_log_tap_without_pid_line() {
        let home = temp_home("log-tap");
        write_exec(&fetch_hy::default_hy_bin(&home), FAKE_HY_LOG);
        std::fs::create_dir_all(home.join(".hy")).unwrap();
        let form = FormState::default();
        std::fs::write(client_yaml_path(&home), config_gen::to_yaml(&form)).unwrap();
        let prepared = prepare_start(&home, &form).unwrap();
        let plan = plan_launch(
            &home,
            &config_gen::to_yaml(&form),
            &prepared,
            Privilege::Root,
        )
        .unwrap();
        let mut proc = spawn_hy(&plan, None, SpawnOpts::default()).expect("root spawn");
        let pid = proc.hy_pid.to_string();
        let mut blob = String::new();
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(50));
            for line in proc.take_log_lines() {
                blob.push_str(&line);
                blob.push('\n');
            }
            if blob.contains("hello-out") && blob.contains("authenticated") {
                break;
            }
        }
        assert!(blob.contains("hello-out"), "{blob}");
        assert!(blob.contains("authenticated"), "{blob}");
        assert!(
            !blob.lines().any(|l| l.trim() == pid),
            "pid line must not enter the log: {blob}"
        );
        proc.stop(Duration::from_secs(3)).ok();
        let _ = std::fs::remove_dir_all(&home);
    }
}
