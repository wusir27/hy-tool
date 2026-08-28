//! TUN interface byte counters (design §6.2 / §9).
//!
//! Polarity is hard-coded, do not invert:
//! - TUN 出 (toward server / out) = Linux rx_bytes / Darwin ifi_ibytes
//! - TUN 入 (from server / in)  = Linux tx_bytes / Darwin ifi_obytes
//!
//! Missing iface / sysfs / getifaddrs → None (UI shows "—", never invented 0).
//! Windows: always None.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const HISTORY_LEN: usize = 60;
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Kernel counters for one named iface.
///
/// `rx_bytes` / ifi_ibytes = bytes the kernel handed to TUN (hy reads the fd) = 出.
/// `tx_bytes` / ifi_obytes = bytes hy wrote to TUN (kernel delivers to host) = 入.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

impl IfCounters {
    pub fn out_bytes(self) -> u64 {
        self.rx_bytes
    }

    pub fn in_bytes(self) -> u64 {
        self.tx_bytes
    }
}

pub fn parse_counter(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok()
}

fn valid_iface_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.contains('/') && !name.contains('\0') && name != "." && name != ".."
}

fn read_counter_file(path: &Path) -> Option<u64> {
    let body = std::fs::read_to_string(path).ok()?;
    parse_counter(&body)
}

/// Linux sysfs reader with injectable base (`/sys/class/net` in production).
pub fn read_linux_at(base: &Path, name: &str) -> Option<IfCounters> {
    if !valid_iface_name(name) {
        return None;
    }
    let name = name.trim();
    let dir = base.join(name).join("statistics");
    let rx = read_counter_file(&dir.join("rx_bytes"))?;
    let tx = read_counter_file(&dir.join("tx_bytes"))?;
    Some(IfCounters {
        rx_bytes: rx,
        tx_bytes: tx,
    })
}

pub fn linux_sysfs_base() -> PathBuf {
    PathBuf::from("/sys/class/net")
}

#[cfg(target_os = "macos")]
fn read_darwin(name: &str) -> Option<IfCounters> {
    if !valid_iface_name(name) {
        return None;
    }
    let name = name.trim();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return None;
        }
        let mut cur = ifap;
        let mut found = None;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_name.is_null() {
                let n = std::ffi::CStr::from_ptr(ifa.ifa_name);
                let is_link =
                    !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family as i32 == libc::AF_LINK;
                if n.to_bytes() == name.as_bytes() && is_link && !ifa.ifa_data.is_null() {
                    let data = &*(ifa.ifa_data as *const libc::if_data);
                    found = Some(IfCounters {
                        rx_bytes: data.ifi_ibytes as u64,
                        tx_bytes: data.ifi_obytes as u64,
                    });
                    break;
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
        found
    }
}

/// Read TUN counters for `name`. Missing iface → None. Windows → None.
pub fn read_iface(name: &str) -> Option<IfCounters> {
    #[cfg(target_os = "linux")]
    {
        read_linux_at(&linux_sysfs_base(), name)
    }
    #[cfg(target_os = "macos")]
    {
        read_darwin(name)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = name;
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RateSnapshot {
    pub out_bps: Option<u64>,
    pub in_bps: Option<u64>,
    pub cum_out: Option<u64>,
    pub cum_in: Option<u64>,
}

impl RateSnapshot {
    pub fn unavailable() -> Self {
        Self {
            out_bps: None,
            in_bps: None,
            cum_out: None,
            cum_in: None,
        }
    }
}

/// 1 Hz sampler with 60-slot (out_bps, in_bps) history.
#[derive(Debug, Clone)]
pub struct IfStats {
    last_attempt: Option<Instant>,
    last: Option<(Instant, IfCounters)>,
    first: Option<IfCounters>,
    out_bps: Option<u64>,
    in_bps: Option<u64>,
    cum_out: Option<u64>,
    cum_in: Option<u64>,
    history: VecDeque<(u64, u64)>,
    live: bool,
}

impl Default for IfStats {
    fn default() -> Self {
        Self::new()
    }
}

impl IfStats {
    pub fn new() -> Self {
        Self {
            last_attempt: None,
            last: None,
            first: None,
            out_bps: None,
            in_bps: None,
            cum_out: None,
            cum_in: None,
            history: VecDeque::with_capacity(HISTORY_LEN),
            live: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn due(&self, now: Instant, interval: Duration) -> bool {
        match self.last_attempt {
            None => true,
            Some(t) => now.saturating_duration_since(t) >= interval,
        }
    }

    /// Inject counters (or None if the iface is gone). `now` is a fake clock in tests.
    pub fn sample(&mut self, now: Instant, counters: Option<IfCounters>) {
        self.last_attempt = Some(now);
        let Some(c) = counters else {
            self.live = false;
            self.last = None;
            self.out_bps = None;
            self.in_bps = None;
            self.cum_out = None;
            self.cum_in = None;
            return;
        };
        self.live = true;
        if self.first.is_none() {
            self.first = Some(c);
        }
        if let Some(first) = self.first {
            self.cum_out = Some(c.out_bytes().saturating_sub(first.out_bytes()));
            self.cum_in = Some(c.in_bytes().saturating_sub(first.in_bytes()));
        }
        if let Some((t0, prev)) = self.last {
            let dt = now.saturating_duration_since(t0).as_secs_f64();
            if dt > 0.0 {
                let d_out = c.out_bytes().saturating_sub(prev.out_bytes());
                let d_in = c.in_bytes().saturating_sub(prev.in_bytes());
                let out_bps = (d_out as f64 / dt).round() as u64;
                let in_bps = (d_in as f64 / dt).round() as u64;
                self.out_bps = Some(out_bps);
                self.in_bps = Some(in_bps);
                self.history.push_back((out_bps, in_bps));
                while self.history.len() > HISTORY_LEN {
                    self.history.pop_front();
                }
            }
        }
        self.last = Some((now, c));
    }

    pub fn snapshot(&self) -> RateSnapshot {
        if !self.live {
            return RateSnapshot::unavailable();
        }
        RateSnapshot {
            out_bps: self.out_bps,
            in_bps: self.in_bps,
            cum_out: self.cum_out,
            cum_in: self.cum_in,
        }
    }

    pub fn out_history(&self) -> Vec<u64> {
        self.history.iter().map(|(o, _)| *o).collect()
    }

    pub fn in_history(&self) -> Vec<u64> {
        self.history.iter().map(|(_, i)| *i).collect()
    }
}

pub fn format_bytes(n: u64) -> String {
    const KB: f64 = 1000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let x = n as f64;
    if x >= GB {
        format!("{:.1} GB", x / GB)
    } else if x >= MB {
        format!("{:.1} MB", x / MB)
    } else if x >= KB {
        format!("{:.1} KB", x / KB)
    } else {
        format!("{n} B")
    }
}

pub fn format_rate(bps: Option<u64>) -> String {
    match bps {
        None => "—".into(),
        Some(n) => format!("{}/s", format_bytes(n)),
    }
}

pub fn format_total(bytes: Option<u64>) -> String {
    match bytes {
        None => "—".into(),
        Some(n) => format_bytes(n),
    }
}

pub fn sparkline(vals: &[u64]) -> String {
    const BLOCKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if vals.is_empty() {
        return String::new();
    }
    let mut max = 1u64;
    for &v in vals {
        if v > max {
            max = v;
        }
    }
    let last = BLOCKS.len() as u64 - 1;
    vals.iter()
        .map(|&v| {
            let idx = v.saturating_mul(last) / max;
            BLOCKS[idx as usize]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write_sysfs(root: &Path, name: &str, rx: &str, tx: &str) {
        let dir = root.join(name).join("statistics");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rx_bytes"), rx).unwrap();
        std::fs::write(dir.join("tx_bytes"), tx).unwrap();
    }

    #[test]
    fn parse_sysfs_decimal_and_missing_path_is_none() {
        assert_eq!(parse_counter("12345\n"), Some(12345));
        assert_eq!(parse_counter("  9  "), Some(9));
        assert_eq!(parse_counter(""), None);
        assert_eq!(parse_counter("nope"), None);

        let root =
            std::env::temp_dir().join(format!("hy-tui-ifstats-{}-{}", std::process::id(), "parse"));
        let _ = std::fs::remove_dir_all(&root);
        write_sysfs(&root, "hy0", "100\n", "200\n");
        let got = read_linux_at(&root, "hy0").unwrap();
        assert_eq!(got.rx_bytes, 100);
        assert_eq!(got.tx_bytes, 200);
        assert!(read_linux_at(&root, "missing").is_none());
        assert!(read_linux_at(&root, "../etc").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_samples_rate_is_delta_over_dt() {
        let mut s = IfStats::new();
        let t0 = Instant::now();
        s.sample(
            t0,
            Some(IfCounters {
                rx_bytes: 1_000,
                tx_bytes: 2_000,
            }),
        );
        s.sample(
            t0 + Duration::from_secs(2),
            Some(IfCounters {
                rx_bytes: 5_000,
                tx_bytes: 8_000,
            }),
        );
        let snap = s.snapshot();
        assert_eq!(snap.out_bps, Some(2_000)); // (5000-1000)/2
        assert_eq!(snap.in_bps, Some(3_000)); // (8000-2000)/2
        assert_eq!(snap.cum_out, Some(4_000));
        assert_eq!(snap.cum_in, Some(6_000));
    }

    #[test]
    fn polarity_rx_ifi_ibytes_is_out_tx_ifi_obytes_is_in() {
        let mut s = IfStats::new();
        let t0 = Instant::now();
        s.sample(
            t0,
            Some(IfCounters {
                rx_bytes: 10,
                tx_bytes: 100,
            }),
        );
        s.sample(
            t0 + Duration::from_secs(1),
            Some(IfCounters {
                rx_bytes: 30,
                tx_bytes: 140,
            }),
        );
        let snap = s.snapshot();
        // rx/ifi_ibytes → 出; tx/ifi_obytes → 入
        assert_eq!(snap.out_bps, Some(20));
        assert_eq!(snap.in_bps, Some(40));
        assert_eq!(
            IfCounters {
                rx_bytes: 1,
                tx_bytes: 2
            }
            .out_bytes(),
            1
        );
        assert_eq!(
            IfCounters {
                rx_bytes: 1,
                tx_bytes: 2
            }
            .in_bytes(),
            2
        );
    }

    #[test]
    fn missing_iface_is_dashes_not_zero() {
        let mut s = IfStats::new();
        let t0 = Instant::now();
        s.sample(
            t0,
            Some(IfCounters {
                rx_bytes: 100,
                tx_bytes: 100,
            }),
        );
        s.sample(
            t0 + Duration::from_secs(1),
            Some(IfCounters {
                rx_bytes: 200,
                tx_bytes: 200,
            }),
        );
        assert_eq!(s.snapshot().out_bps, Some(100));
        s.sample(t0 + Duration::from_secs(2), None);
        let snap = s.snapshot();
        assert!(snap.out_bps.is_none());
        assert!(snap.in_bps.is_none());
        assert!(snap.cum_out.is_none());
        assert!(snap.cum_in.is_none());
        assert_eq!(format_rate(snap.out_bps), "—");
        assert_eq!(format_total(snap.cum_out), "—");
        assert_ne!(format_rate(snap.out_bps), "0");
        assert_ne!(format_total(snap.cum_out), "0 B");
        assert!(read_iface("").is_none());
        assert!(read_linux_at(Path::new("/no/such/sysfs"), "hy0").is_none());
    }
}
