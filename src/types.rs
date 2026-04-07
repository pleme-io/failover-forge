//! Failover drill domain types — the source-of-truth result struct that
//! downstream consumers (manifest builder, report renderer, Confluence
//! publisher) reference.

use serde::{Deserialize, Serialize};

use crate::canary::ProbeResult;

/// Aggregated result of a failover drill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverDrillResult {
    pub timestamp: String,
    pub target: String,
    pub direction: String,
    pub duration_secs: u64,
    pub probe_interval_secs: u64,
    pub total_probes: usize,
    pub success_probes: usize,
    pub failure_probes: usize,
    pub failover_detected: bool,
    pub first_failure_at: Option<String>,
    pub first_recovery_at: Option<String>,
    pub gap_secs: Option<u64>,
    pub baseline_secs: u64,
    pub sla_target_secs: u64,
    pub sla_met: bool,
    pub ip_changes: Vec<IpChange>,
    pub status_code_distribution: Vec<StatusCodeCount>,
    pub probes: Vec<ProbeResult>,
    pub error: Option<String>,
}

/// One observed transition between resolved IPs during the probe loop.
/// Multiple changes within a single drill window indicate flapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpChange {
    pub timestamp: String,
    pub from_ip: String,
    pub to_ip: String,
}

/// Aggregate count of probe results by HTTP status code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCodeCount {
    pub status_code: u16,
    pub count: usize,
}
