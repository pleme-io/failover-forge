//! Failover drill manifest — load-bearing metadata included in the tarball.
//!
//! Carries the join-key fields (drill_id, target, direction) and the
//! verdict + SLA metrics so a downstream consumer can identify and
//! classify the drill without parsing the full event stream or
//! `FailoverDrillResult`.

use serde::{Deserialize, Serialize};

use drill_core::events::{EventContext, Verdict, EVENT_SCHEMA_VERSION};

use crate::types::FailoverDrillResult;

/// Tarball layout version. Bumped on incompatible changes.
pub const TARBALL_FORMAT_VERSION: &str = "1.0.0";

/// Drill manifest written to the root of the tarball as `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverManifest {
    pub drill_id: String,
    pub schema_version: String,
    pub tarball_format_version: String,
    pub drill_kind: String,
    pub target: String,
    pub direction: String,
    pub started_at: String,
    pub completed_at: String,
    pub total_probes: usize,
    pub success_probes: usize,
    pub failure_probes: usize,
    pub gap_secs: Option<u64>,
    pub sla_target_secs: u64,
    pub sla_met: bool,
    pub verdict: Verdict,
    pub events_count: usize,
}

impl FailoverManifest {
    /// Build a manifest from a completed drill result.
    #[must_use]
    pub fn from_drill(
        ctx: &EventContext,
        result: &FailoverDrillResult,
        events_count: usize,
        started_at: &str,
        completed_at: &str,
    ) -> Self {
        let verdict = if result.sla_met {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        Self {
            drill_id: ctx.drill_id.clone(),
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            tarball_format_version: TARBALL_FORMAT_VERSION.to_string(),
            drill_kind: "failover".to_string(),
            target: result.target.clone(),
            direction: result.direction.clone(),
            started_at: started_at.to_string(),
            completed_at: completed_at.to_string(),
            total_probes: result.total_probes,
            success_probes: result.success_probes,
            failure_probes: result.failure_probes,
            gap_secs: result.gap_secs,
            sla_target_secs: result.sla_target_secs,
            sla_met: result.sla_met,
            verdict,
            events_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_result() -> FailoverDrillResult {
        FailoverDrillResult {
            timestamp: "2026-04-06T12:00:00Z".to_string(),
            target: "vault.staging.akeyless.dev".to_string(),
            direction: "master-to-read".to_string(),
            duration_secs: 300,
            probe_interval_secs: 1,
            total_probes: 300,
            success_probes: 240,
            failure_probes: 60,
            failover_detected: true,
            first_failure_at: Some("2026-04-06T12:01:00Z".to_string()),
            first_recovery_at: Some("2026-04-06T12:01:57Z".to_string()),
            gap_secs: Some(57),
            baseline_secs: 60,
            sla_target_secs: 60,
            sla_met: true,
            ip_changes: vec![],
            status_code_distribution: vec![],
            probes: vec![],
            error: None,
        }
    }

    #[test]
    fn manifest_passing_drill_has_pass_verdict() {
        let ctx = EventContext::new(
            "staging".to_string(),
            "n/a".to_string(),
            "staging".to_string(),
        );
        let result = fake_result();
        let manifest = FailoverManifest::from_drill(
            &ctx,
            &result,
            10,
            "2026-04-06T12:00:00Z",
            "2026-04-06T12:05:00Z",
        );
        assert_eq!(manifest.verdict, Verdict::Pass);
        assert_eq!(manifest.gap_secs, Some(57));
        assert!(manifest.sla_met);
        assert_eq!(manifest.drill_kind, "failover");
        assert_eq!(manifest.target, "vault.staging.akeyless.dev");
        assert!(manifest.drill_id.starts_with("staging-n/a-staging-"));
    }

    #[test]
    fn manifest_failing_drill_has_fail_verdict() {
        let ctx = EventContext::new(
            "staging".to_string(),
            "n/a".to_string(),
            "staging".to_string(),
        );
        let mut result = fake_result();
        result.gap_secs = Some(120);
        result.sla_met = false;
        let manifest = FailoverManifest::from_drill(
            &ctx,
            &result,
            10,
            "2026-04-06T12:00:00Z",
            "2026-04-06T12:05:00Z",
        );
        assert_eq!(manifest.verdict, Verdict::Fail);
    }
}
