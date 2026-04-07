//! Continuous HTTP probe loop for failover gap measurement.
//!
//! Hits a target URL on a fixed interval, capturing per-probe timing,
//! status code, and resolved IP. The probe loop runs for a configurable
//! duration; per-probe results are collected and analyzed downstream
//! (see `drill::analyze`) to detect the failover gap.

use std::process::Command;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Single HTTP probe result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub timestamp: String,
    pub status_code: u16,
    pub latency_ms: u64,
    pub remote_ip: String,
    pub error: Option<String>,
}

impl ProbeResult {
    /// HTTP 2xx and 3xx are considered successful for failover detection.
    /// Anything else (including 0 / connection error) is a failure.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 400
    }
}

/// Issue a single HTTP probe to `url` via curl.
///
/// Captures: status code, resolved remote IP, latency in ms.
/// Uses `-k` to accept self-signed certs and `--max-time 5` for a hard
/// per-request timeout (so a hanging endpoint doesn't stall the loop).
#[must_use]
pub fn probe(url: &str) -> ProbeResult {
    let start = Instant::now();
    let timestamp = Utc::now().to_rfc3339();

    let output = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code} %{remote_ip}",
            "-k",
            "--max-time",
            "5",
            url,
        ])
        .output();

    let latency_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = stdout.trim().split_whitespace().collect();
            let status_code: u16 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let remote_ip = parts.get(1).copied().unwrap_or("").to_string();
            ProbeResult {
                timestamp,
                status_code,
                latency_ms,
                remote_ip,
                error: None,
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            ProbeResult {
                timestamp,
                status_code: 0,
                latency_ms,
                remote_ip: String::new(),
                error: Some(stderr),
            }
        }
        Err(e) => ProbeResult {
            timestamp,
            status_code: 0,
            latency_ms,
            remote_ip: String::new(),
            error: Some(e.to_string()),
        },
    }
}

/// Run a continuous probe loop against `url` for `duration`, polling at `interval`.
///
/// Calls `on_probe` for each result (so the caller can stream live logs)
/// and returns the full vector of results when the duration elapses.
pub fn run_loop(
    url: &str,
    duration: Duration,
    interval: Duration,
    mut on_probe: impl FnMut(&ProbeResult),
) -> Vec<ProbeResult> {
    let start = Instant::now();
    let mut results: Vec<ProbeResult> = Vec::new();

    while start.elapsed() < duration {
        let result = probe(url);
        on_probe(&result);
        results.push(result);
        std::thread::sleep(interval);
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_result_success_is_2xx_or_3xx() {
        let mut p = ProbeResult {
            timestamp: "now".to_string(),
            status_code: 200,
            latency_ms: 10,
            remote_ip: String::new(),
            error: None,
        };
        assert!(p.is_success());
        p.status_code = 301;
        assert!(p.is_success());
        p.status_code = 503;
        assert!(!p.is_success());
        p.status_code = 0;
        assert!(!p.is_success());
    }
}
