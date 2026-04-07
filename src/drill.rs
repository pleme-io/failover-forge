//! Failover drill orchestration.
//!
//! End-to-end drill: preconditions → optional trigger → probe loop →
//! analysis → tarball assembly. Emits structured `drill_core::events`
//! through every state transition for shinryū consumption.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use drill_core::events::{self, DrillEvent, EventContext, Mode, Phase, Verdict};

use crate::canary::{self, ProbeResult};
use crate::manifest::FailoverManifest;
use crate::report;
use crate::types::{FailoverDrillResult, IpChange, StatusCodeCount};

/// CLI args for the failover drill.
pub struct DrillArgs {
    pub target: String,
    pub direction: String,
    pub duration: Duration,
    pub interval: Duration,
    pub workspace: PathBuf,
    pub output: PathBuf,
    pub tenant: String,
    pub trigger_gh_workflow: Option<String>,
    pub sla_target_secs: u64,
}

/// Run the failover drill end-to-end.
///
/// # Errors
///
/// Returns an error if the workspace cannot be created, the baseline probe
/// fails, or tarball assembly fails. The drill always emits a
/// `DrillCompleted` or `DrillFailed` event before returning.
pub fn run(args: &DrillArgs) -> anyhow::Result<()> {
    let drill_start = Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();

    std::fs::create_dir_all(&args.workspace)?;
    std::fs::create_dir_all(&args.output)?;
    let events_file = args.workspace.join("events.ndjson");
    let _ = std::fs::remove_file(&events_file);

    // failover drills always run against staging — no per-tenant variance
    // (the target hostname encodes the env). cloud is "n/a" for failover.
    let ctx = EventContext::new(
        args.tenant.clone(),
        "n/a".to_string(),
        "staging".to_string(),
    )
    .with_events_file_path(events_file.clone());

    let probe_url = format!("https://{}/health", args.target);

    events::emit(
        &ctx,
        DrillEvent::DrillStarted {
            mode: Mode::Drill,
            restore_time: "n/a (failover drill)".to_string(),
            app_version: format!("failover-{}", args.direction),
            terraform_path: probe_url.clone(),
        },
    );

    // ── Phase 1: PRECONDITIONS ──────────────────────────────────────────
    events::emit(
        &ctx,
        DrillEvent::PhaseStarted {
            phase: Phase::Preconditions,
        },
    );
    let phase1_start = Instant::now();
    let baseline = canary::probe(&probe_url);
    let p1_passed = baseline.is_success();
    events::emit(
        &ctx,
        DrillEvent::GateChecked {
            phase: Phase::Preconditions,
            gate: "[Gate 1]".to_string(),
            passed: p1_passed,
            message: format!("baseline probe HTTP {}", baseline.status_code),
            expected: "HTTP 2xx/3xx".to_string(),
            actual: format!("HTTP {}", baseline.status_code),
        },
    );
    events::emit(
        &ctx,
        DrillEvent::PhaseCompleted {
            phase: Phase::Preconditions,
            duration_ms: u64::try_from(phase1_start.elapsed().as_millis()).unwrap_or(0),
            passed: p1_passed,
        },
    );
    if !p1_passed {
        events::emit(
            &ctx,
            DrillEvent::DrillFailed {
                phase: Phase::Preconditions,
                error: format!(
                    "baseline probe failed: HTTP {} (target {})",
                    baseline.status_code, args.target
                ),
            },
        );
        anyhow::bail!(
            "Baseline probe to {} failed (HTTP {}); cannot run failover drill",
            probe_url,
            baseline.status_code
        );
    }

    // ── Phase 2: TRIGGER ────────────────────────────────────────────────
    events::emit(
        &ctx,
        DrillEvent::PhaseStarted {
            phase: Phase::Trigger,
        },
    );
    let trigger_start = Instant::now();
    if let Some(workflow) = &args.trigger_gh_workflow {
        println!("Triggering GH workflow: {workflow}");
        let _ = Command::new("gh")
            .args(["workflow", "run", workflow])
            .spawn();
    }
    events::emit(
        &ctx,
        DrillEvent::PhaseCompleted {
            phase: Phase::Trigger,
            duration_ms: u64::try_from(trigger_start.elapsed().as_millis()).unwrap_or(0),
            passed: true,
        },
    );

    // ── Phase 3: RESTORE (canary loop captures the failover gap) ────────
    events::emit(
        &ctx,
        DrillEvent::PhaseStarted {
            phase: Phase::Restore,
        },
    );
    let restore_start = Instant::now();
    let probes = canary::run_loop(&probe_url, args.duration, args.interval, |p| {
        eprintln!(
            "probe: {} HTTP {} {}ms ip={}",
            p.timestamp, p.status_code, p.latency_ms, p.remote_ip
        );
    });
    events::emit(
        &ctx,
        DrillEvent::PhaseCompleted {
            phase: Phase::Restore,
            duration_ms: u64::try_from(restore_start.elapsed().as_millis()).unwrap_or(0),
            passed: true,
        },
    );

    // ── Phase 4: VERIFICATION (analyze probes for gap + SLA) ────────────
    events::emit(
        &ctx,
        DrillEvent::PhaseStarted {
            phase: Phase::Verification,
        },
    );
    let verify_start = Instant::now();
    let result = analyze(&probes, args, started_at.clone());
    let verify_passed = result.sla_met;
    events::emit(
        &ctx,
        DrillEvent::GateChecked {
            phase: Phase::Verification,
            gate: "[Gate 2]".to_string(),
            passed: verify_passed,
            message: format!(
                "failover gap = {}s (SLA target {}s)",
                result.gap_secs.unwrap_or(0),
                result.sla_target_secs
            ),
            expected: format!("≤ {}s", result.sla_target_secs),
            actual: format!("{}s", result.gap_secs.unwrap_or(0)),
        },
    );
    events::emit(
        &ctx,
        DrillEvent::PhaseCompleted {
            phase: Phase::Verification,
            duration_ms: u64::try_from(verify_start.elapsed().as_millis()).unwrap_or(0),
            passed: verify_passed,
        },
    );

    // ── Drill completed ─────────────────────────────────────────────────
    let verdict = if verify_passed {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    let total_ms = u64::try_from(drill_start.elapsed().as_millis()).unwrap_or(0);
    events::emit(
        &ctx,
        DrillEvent::DrillCompleted {
            verdict,
            total_duration_ms: total_ms,
            measured_rto_secs: result.gap_secs.unwrap_or(0),
        },
    );

    let completed_at = chrono::Utc::now().to_rfc3339();

    // ── Build manifest, render report, assemble tarball ─────────────────
    let events_count = std::fs::read_to_string(&events_file)
        .map(|s| s.lines().count())
        .unwrap_or(0);

    let drill_manifest = FailoverManifest::from_drill(
        &ctx,
        &result,
        events_count,
        &started_at,
        &completed_at,
    );

    let report_md = report::render(&result, &drill_manifest);

    // Write artifacts to staging dir.
    let manifest_json = serde_json::to_string_pretty(&drill_manifest)?;
    std::fs::write(args.workspace.join("manifest.json"), manifest_json)?;

    let result_json = serde_json::to_string_pretty(&result)?;
    std::fs::write(args.workspace.join("drill-result.json"), result_json)?;

    std::fs::write(args.workspace.join("report.md"), &report_md)?;

    // Tar everything in the workspace dir.
    let tarball_path = args.output.join(format!("{}.tar.gz", ctx.drill_id));
    let tar_output = Command::new("tar")
        .arg("-czf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(&args.workspace)
        .arg(".")
        .output()?;
    if !tar_output.status.success() {
        anyhow::bail!(
            "tar -czf {} failed: {}",
            tarball_path.display(),
            String::from_utf8_lossy(&tar_output.stderr).trim()
        );
    }

    println!(
        "Failover drill complete. Tarball: {}",
        tarball_path.display()
    );
    println!("Verdict: {verdict:?}");
    println!(
        "Gap: {}s (SLA target: {}s, met: {})",
        result.gap_secs.unwrap_or(0),
        result.sla_target_secs,
        result.sla_met
    );

    Ok(())
}

/// Analyze probe results to detect failover gap and SLA compliance.
#[allow(clippy::too_many_lines)]
fn analyze(probes: &[ProbeResult], args: &DrillArgs, started_at: String) -> FailoverDrillResult {
    let total = probes.len();
    let success = probes.iter().filter(|p| p.is_success()).count();
    let failure = total - success;

    let first_failure_idx = probes.iter().position(|p| !p.is_success());
    let first_recovery_idx = first_failure_idx.and_then(|i| {
        probes[i..]
            .iter()
            .position(|p| p.is_success())
            .map(|j| i + j)
    });

    let first_failure_at = first_failure_idx.map(|i| probes[i].timestamp.clone());
    let first_recovery_at = first_recovery_idx.map(|i| probes[i].timestamp.clone());

    let gap_secs = match (&first_failure_at, &first_recovery_at) {
        (Some(start), Some(end)) => {
            let s = chrono::DateTime::parse_from_rfc3339(start).ok();
            let e = chrono::DateTime::parse_from_rfc3339(end).ok();
            match (s, e) {
                (Some(s), Some(e)) => {
                    let secs = (e - s).num_seconds().max(0);
                    Some(u64::try_from(secs).unwrap_or(0))
                }
                _ => None,
            }
        }
        _ => None,
    };

    let failover_detected = first_failure_idx.is_some();
    let sla_met = match gap_secs {
        Some(g) => g <= args.sla_target_secs,
        None => !failover_detected, // no failover at all = SLA trivially met
    };

    // IP change detection.
    let mut ip_changes = Vec::new();
    let mut prev_ip = String::new();
    for probe in probes {
        if !probe.remote_ip.is_empty() && probe.remote_ip != prev_ip {
            if !prev_ip.is_empty() {
                ip_changes.push(IpChange {
                    timestamp: probe.timestamp.clone(),
                    from_ip: prev_ip.clone(),
                    to_ip: probe.remote_ip.clone(),
                });
            }
            prev_ip.clone_from(&probe.remote_ip);
        }
    }

    // Status code distribution.
    let mut status_counts: std::collections::BTreeMap<u16, usize> =
        std::collections::BTreeMap::new();
    for probe in probes {
        *status_counts.entry(probe.status_code).or_insert(0) += 1;
    }
    let status_code_distribution: Vec<StatusCodeCount> = status_counts
        .into_iter()
        .map(|(status_code, count)| StatusCodeCount { status_code, count })
        .collect();

    FailoverDrillResult {
        timestamp: started_at,
        target: args.target.clone(),
        direction: args.direction.clone(),
        duration_secs: args.duration.as_secs(),
        probe_interval_secs: args.interval.as_secs(),
        total_probes: total,
        success_probes: success,
        failure_probes: failure,
        failover_detected,
        first_failure_at,
        first_recovery_at,
        gap_secs,
        baseline_secs: 60,
        sla_target_secs: args.sla_target_secs,
        sla_met,
        ip_changes,
        status_code_distribution,
        probes: probes.to_vec(),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canary::ProbeResult;

    fn make_probe(ts: &str, status: u16, ip: &str) -> ProbeResult {
        ProbeResult {
            timestamp: ts.to_string(),
            status_code: status,
            latency_ms: 10,
            remote_ip: ip.to_string(),
            error: None,
        }
    }

    fn fake_args() -> DrillArgs {
        DrillArgs {
            target: "vault.staging.akeyless.dev".to_string(),
            direction: "master-to-read".to_string(),
            duration: Duration::from_secs(60),
            interval: Duration::from_secs(1),
            workspace: PathBuf::from("/tmp"),
            output: PathBuf::from("/tmp"),
            tenant: "staging".to_string(),
            trigger_gh_workflow: None,
            sla_target_secs: 60,
        }
    }

    #[test]
    fn analyze_detects_failover_gap_and_ip_change() {
        let probes = vec![
            make_probe("2026-04-06T12:00:00Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:01Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:02Z", 503, "1.2.3.4"),
            make_probe("2026-04-06T12:00:50Z", 503, "1.2.3.4"),
            make_probe("2026-04-06T12:00:59Z", 200, "5.6.7.8"),
            make_probe("2026-04-06T12:01:00Z", 200, "5.6.7.8"),
        ];
        let result = analyze(&probes, &fake_args(), "2026-04-06T12:00:00Z".to_string());
        assert!(result.failover_detected);
        assert_eq!(result.gap_secs, Some(57));
        assert!(result.sla_met);
        assert_eq!(result.ip_changes.len(), 1);
        assert_eq!(result.ip_changes[0].from_ip, "1.2.3.4");
        assert_eq!(result.ip_changes[0].to_ip, "5.6.7.8");
        assert_eq!(result.success_probes, 4);
        assert_eq!(result.failure_probes, 2);
    }

    #[test]
    fn analyze_no_failover_passes_sla_trivially() {
        let probes = vec![
            make_probe("2026-04-06T12:00:00Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:01Z", 200, "1.2.3.4"),
        ];
        let result = analyze(&probes, &fake_args(), "2026-04-06T12:00:00Z".to_string());
        assert!(!result.failover_detected);
        assert!(result.sla_met);
        assert_eq!(result.gap_secs, None);
    }

    #[test]
    fn analyze_failover_exceeding_sla_fails() {
        let probes = vec![
            make_probe("2026-04-06T12:00:00Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:01Z", 503, "1.2.3.4"),
            make_probe("2026-04-06T12:02:30Z", 200, "5.6.7.8"), // 149s gap
        ];
        let mut args = fake_args();
        args.sla_target_secs = 60;
        let result = analyze(&probes, &args, "2026-04-06T12:00:00Z".to_string());
        assert!(result.failover_detected);
        assert_eq!(result.gap_secs, Some(149));
        assert!(!result.sla_met);
    }

    #[test]
    fn analyze_status_code_distribution() {
        let probes = vec![
            make_probe("2026-04-06T12:00:00Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:01Z", 200, "1.2.3.4"),
            make_probe("2026-04-06T12:00:02Z", 503, "1.2.3.4"),
            make_probe("2026-04-06T12:00:03Z", 503, "1.2.3.4"),
            make_probe("2026-04-06T12:00:04Z", 0, ""),
        ];
        let result = analyze(&probes, &fake_args(), "2026-04-06T12:00:00Z".to_string());
        let dist: std::collections::HashMap<u16, usize> = result
            .status_code_distribution
            .into_iter()
            .map(|c| (c.status_code, c.count))
            .collect();
        assert_eq!(dist.get(&200), Some(&2));
        assert_eq!(dist.get(&503), Some(&2));
        assert_eq!(dist.get(&0), Some(&1));
    }
}
