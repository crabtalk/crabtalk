//! Gateway clients for cross-framework benchmarks.
//!
//! Each gateway sends a task prompt to a framework and collects the response.

pub mod crabtalk;
pub mod hermes;
pub mod openclaw;
pub mod opencode;

use crate::task::Task;
use std::{
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

/// Result of running a task through a framework.
pub struct TaskResult {
    /// Whether the task completed successfully.
    pub success: bool,
    /// The framework's text response.
    pub response: String,
    /// Wall-clock time in milliseconds.
    pub wall_clock_ms: u64,
    /// Peak resident set size in bytes (self process, before/after delta).
    pub peak_rss_bytes: Option<u64>,
}

/// Common interface for sending tasks to agent frameworks.
pub trait Gateway {
    /// Send a task prompt and collect the response. Blocking (for Criterion).
    fn run_task(&self, rt: &tokio::runtime::Runtime, task: &Task) -> TaskResult;
}

/// Check if a TCP port is reachable (2s timeout).
pub fn check_reachable(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

/// Time a future, measure peak RSS, and wrap the result in a TaskResult.
pub async fn timed<F, Fut>(f: F) -> TaskResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let rss_before = peak_rss_bytes();
    let start = Instant::now();
    let result = f().await;
    let wall_clock_ms = start.elapsed().as_millis() as u64;
    let rss_after = peak_rss_bytes();
    let peak_rss_bytes = rss_before.zip(rss_after).map(|(b, a)| a.saturating_sub(b));

    match result {
        Ok(response) => TaskResult {
            success: true,
            response,
            wall_clock_ms,
            peak_rss_bytes,
        },
        Err(e) => TaskResult {
            success: false,
            response: e,
            wall_clock_ms,
            peak_rss_bytes,
        },
    }
}

/// Get the current process peak RSS in bytes.
#[cfg(target_os = "macos")]
fn peak_rss_bytes() -> Option<u64> {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if ret == 0 {
        // macOS reports ru_maxrss in bytes.
        Some(usage.ru_maxrss as u64)
    } else {
        None
    }
}

/// Get the current process peak RSS in bytes.
#[cfg(target_os = "linux")]
fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(kb) = line.strip_prefix("VmHWM:") {
            let kb = kb.trim().strip_suffix("kB")?.trim();
            return kb.parse::<u64>().ok().map(|v| v * 1024);
        }
    }
    None
}

/// Fallback for other platforms.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peak_rss_bytes() -> Option<u64> {
    None
}
