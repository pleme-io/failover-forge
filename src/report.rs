//! Markdown report renderer for failover drills.
//!
//! Mirrors the Q1 2025 Confluence drill page structure: drill identity
//! header, failover gap measurement table, IP change table (when present),
//! status code distribution. Lives inside the tarball as `report.md`.

use std::fmt::Write;

use crate::manifest::FailoverManifest;
use crate::types::FailoverDrillResult;

/// Render a markdown report from the drill result + manifest.
#[must_use]
pub fn render(result: &FailoverDrillResult, manifest: &FailoverManifest) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# Failover Drill Report");
    let _ = writeln!(md);
    let _ = writeln!(md, "**Drill ID:** `{}`", manifest.drill_id);
    let _ = writeln!(md, "**Target:** {}", manifest.target);
    let _ = writeln!(md, "**Direction:** {}", manifest.direction);
    let _ = writeln!(md, "**Started:** {}", manifest.started_at);
    let _ = writeln!(md, "**Completed:** {}", manifest.completed_at);
    let _ = writeln!(md, "**Verdict:** {:?}", manifest.verdict);
    let _ = writeln!(md);

    let _ = writeln!(md, "## Failover Gap Measurement");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Metric | Value |");
    let _ = writeln!(md, "|--------|-------|");
    let _ = writeln!(md, "| Total probes | {} |", manifest.total_probes);
    let _ = writeln!(md, "| Successful probes | {} |", manifest.success_probes);
    let _ = writeln!(md, "| Failed probes | {} |", manifest.failure_probes);
    let _ = writeln!(md, "| Failover detected | {} |", result.failover_detected);
    let _ = writeln!(
        md,
        "| First failure at | {} |",
        result.first_failure_at.as_deref().unwrap_or("n/a")
    );
    let _ = writeln!(
        md,
        "| First recovery at | {} |",
        result.first_recovery_at.as_deref().unwrap_or("n/a")
    );
    let _ = writeln!(
        md,
        "| Gap (seconds) | {} |",
        result
            .gap_secs
            .map_or_else(|| "n/a".to_string(), |g| g.to_string())
    );
    let _ = writeln!(md, "| SLA target | {}s |", manifest.sla_target_secs);
    let _ = writeln!(
        md,
        "| SLA met | {} |",
        if manifest.sla_met { "MET" } else { "MISSED" }
    );
    let _ = writeln!(md);

    if !result.ip_changes.is_empty() {
        let _ = writeln!(md, "## IP Changes (failover transitions)");
        let _ = writeln!(md);
        let _ = writeln!(md, "| Timestamp | From | To |");
        let _ = writeln!(md, "|-----------|------|----|");
        for change in &result.ip_changes {
            let _ = writeln!(
                md,
                "| {} | {} | {} |",
                change.timestamp, change.from_ip, change.to_ip
            );
        }
        let _ = writeln!(md);
    }

    let _ = writeln!(md, "## Status Code Distribution");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Status Code | Count |");
    let _ = writeln!(md, "|-------------|-------|");
    for sc in &result.status_code_distribution {
        let _ = writeln!(md, "| {} | {} |", sc.status_code, sc.count);
    }

    md
}
