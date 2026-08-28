//! Line-wise ring buffer for hy client stdout+stderr (Run tab).
//!
//! Never put the pid line, sudo password, or auth form values here. Callers
//! must not push those.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const LOG_CAP: usize = 2000;

/// Bounded display ring. Oldest lines drop when full.
#[derive(Debug, Clone, Default)]
pub struct LogRing {
    lines: VecDeque<String>,
}

impl LogRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: String) {
        while self.lines.len() >= LOG_CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    pub fn last_n(&self, n: usize) -> impl Iterator<Item = &str> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(start).map(String::as_str)
    }
}

/// Cheap producer/consumer for drain threads → UI poll.
#[derive(Clone, Default)]
pub struct LogTap {
    q: Arc<Mutex<VecDeque<String>>>,
}

impl LogTap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, line: String) {
        let mut q = self.q.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(line);
    }

    pub fn take(&self) -> Vec<String> {
        let mut q = self.q.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).collect()
    }
}

/// Split `data` onto `partial` and emit complete lines (no trailing CR/LF).
pub fn feed_bytes(partial: &mut Vec<u8>, data: &[u8], mut sink: impl FnMut(String)) {
    partial.extend_from_slice(data);
    while let Some(pos) = partial.iter().position(|&b| b == b'\n') {
        let mut line: Vec<u8> = partial.drain(..=pos).collect();
        while matches!(line.last(), Some(&b'\n') | Some(&b'\r')) {
            line.pop();
        }
        sink(String::from_utf8_lossy(&line).into_owned());
    }
}

/// Emit a leftover incomplete line on EOF.
pub fn flush_partial(partial: &mut Vec<u8>, mut sink: impl FnMut(String)) {
    if partial.is_empty() {
        return;
    }
    let mut line = std::mem::take(partial);
    while matches!(line.last(), Some(&b'\n') | Some(&b'\r')) {
        line.pop();
    }
    sink(String::from_utf8_lossy(&line).into_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_cap_drops_oldest_and_clear_empties() {
        let mut r = LogRing::new();
        for i in 0..(LOG_CAP + 1) {
            r.push(i.to_string());
        }
        assert_eq!(r.len(), LOG_CAP);
        assert_eq!(r.iter().next(), Some("1"));
        let last = LOG_CAP.to_string();
        assert_eq!(r.iter().last(), Some(last.as_str()));
        r.clear();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn feed_bytes_splits_lines_and_keeps_partial() {
        let mut partial = Vec::new();
        let mut out = Vec::new();
        feed_bytes(&mut partial, b"a\nb\npar", |s| out.push(s));
        assert_eq!(out, ["a", "b"]);
        assert_eq!(partial, b"par");
        flush_partial(&mut partial, |s| out.push(s));
        assert_eq!(out, ["a", "b", "par"]);
        assert!(partial.is_empty());
    }
}
