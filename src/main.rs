//! failover-forge — Akeyless region failover drill orchestrator.
//!
//! Continuous canary probe loop + (optional) failover trigger + gap
//! measurement + tarball output. Sibling tool to pitr-forge for the
//! ASM-17781 ticket. Both tools share the `drill-core` crate for event
//! schema, drill_id semantics, and JSON-line emission to stderr (Vector
//! kubernetes_logs source picks them up and ships to shinryū).

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions
)]

mod canary;
mod drill;
mod manifest;
mod report;
mod types;

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "failover-forge",
    about = "Akeyless region failover drill orchestrator — continuous canary, gap measurement, structured event emission, tarball output",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a region failover drill.
    ///
    /// Issues a continuous probe loop against the target vault endpoint,
    /// optionally invokes a GH workflow to trigger the failover, then
    /// analyzes the probe results to detect the failover gap and check
    /// it against the SLA target.
    Drill {
        /// Target vault hostname (e.g. vault.staging.akeyless.dev)
        #[arg(long)]
        target: String,
        /// Failover direction: master-to-read | read-to-master
        #[arg(long, default_value = "master-to-read")]
        direction: String,
        /// Drill duration (window for canary). Format: 60s, 5m, 1h
        #[arg(long, default_value = "5m")]
        duration: String,
        /// Probe interval. Format: 1s, 500ms (ms not yet supported, only s/m/h)
        #[arg(long, default_value = "1s")]
        interval: String,
        /// Workspace dir (events.ndjson + tarball staging)
        #[arg(long, default_value = "/tmp/failover-forge")]
        workspace: PathBuf,
        /// Output dir for the drill tarball
        #[arg(long, default_value = "./drill-output")]
        output: PathBuf,
        /// Tenant slug for events (default: staging)
        #[arg(long, default_value = "staging")]
        tenant: String,
        /// Optional: trigger an akeyless GH workflow at drill start (e.g. `api-gw-failover.yaml`)
        #[arg(long)]
        trigger_gh_workflow: Option<String>,
        /// SLA target for the failover gap in seconds (default: 60)
        #[arg(long, default_value = "60")]
        sla_target_secs: u64,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Drill {
            target,
            direction,
            duration,
            interval,
            workspace,
            output,
            tenant,
            trigger_gh_workflow,
            sla_target_secs,
        } => {
            let args = drill::DrillArgs {
                target,
                direction,
                duration: parse_duration(&duration)?,
                interval: parse_duration(&interval)?,
                workspace,
                output,
                tenant,
                trigger_gh_workflow,
                sla_target_secs,
            };
            drill::run(&args)?;
        }
    }

    Ok(())
}

/// Parse a duration string like `60s`, `5m`, `1h` into a `Duration`.
fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    let split_at = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let (num_part, unit) = s.split_at(split_at);
    let num: u64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration number: {num_part}"))?;
    let secs = match unit {
        "s" | "" => num,
        "m" => num * 60,
        "h" => num * 60 * 60,
        other => anyhow::bail!("unknown duration unit: {other} (expected s, m, or h)"),
    };
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn parse_duration_invalid_unit_fails() {
        assert!(parse_duration("5x").is_err());
    }
}
