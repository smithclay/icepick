//! Snapshot commands for listing and cleanup

use crate::cli::catalog::CatalogConfig;
use crate::cli::output::{format_number, print, OutputFormat, Outputable};
use crate::cli::util::parse_table_ident;
use crate::snapshot_cleanup::{
    plan_snapshot_cleanup, CleanupOptions, CleanupPlan, RetentionReason,
};
use chrono::{TimeZone, Utc};
use clap::Subcommand;
use comfy_table::{Row, Table as ComfyTable};
use serde::Serialize;

/// Snapshot commands
#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// List snapshots in a table
    List {
        /// Table identifier (namespace.table)
        table: String,
    },

    /// Cleanup old snapshots based on retention policy
    Cleanup {
        /// Table identifier (namespace.table)
        table: String,

        /// Minimum age in days before a snapshot can be expired
        #[arg(long, default_value = "7")]
        older_than_days: u32,

        /// Minimum number of snapshots to always retain (most recent)
        #[arg(long, default_value = "10")]
        retain_last: usize,

        /// Show plan without executing
        #[arg(long)]
        dry_run: bool,
    },
}

/// Snapshot list output
#[derive(Debug, Serialize)]
pub struct SnapshotList {
    pub table: String,
    pub snapshots: Vec<SnapshotEntry>,
    pub total_count: usize,
    pub current_snapshot_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotEntry {
    pub snapshot_id: i64,
    pub timestamp: String,
    pub age_days: f64,
    pub operation: String,
    pub is_current: bool,
    pub refs: Vec<String>,
}

impl Outputable for SnapshotList {
    fn to_text(&self) -> String {
        if self.snapshots.is_empty() {
            return format!("No snapshots found in table '{}'.", self.table);
        }

        let mut lines = vec![format!("Snapshots in '{}':", self.table), String::new()];

        let mut table = ComfyTable::new();
        table.set_header(Row::from(vec![
            "Snapshot ID",
            "Timestamp",
            "Age",
            "Operation",
            "Current",
            "Refs",
        ]));

        for snapshot in &self.snapshots {
            table.add_row(Row::from(vec![
                snapshot.snapshot_id.to_string(),
                snapshot.timestamp.clone(),
                format_age(snapshot.age_days),
                snapshot.operation.clone(),
                if snapshot.is_current { "yes" } else { "" }.to_string(),
                snapshot.refs.join(", "),
            ]));
        }
        lines.push(table.to_string());

        lines.push(String::new());
        lines.push(format!("Total: {} snapshots", self.total_count));

        lines.join("\n")
    }
}

/// Cleanup plan output
#[derive(Debug, Serialize)]
pub struct CleanupPlanOutput {
    pub table: String,
    pub older_than_days: u32,
    pub retain_last: usize,
    pub total_snapshots: usize,
    pub snapshots_to_remove: Vec<SnapshotToRemove>,
    pub snapshots_to_retain: Vec<SnapshotToRetain>,
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct SnapshotToRemove {
    pub snapshot_id: i64,
    pub timestamp: String,
    pub age_days: f64,
    pub operation: String,
}

#[derive(Debug, Serialize)]
pub struct SnapshotToRetain {
    pub snapshot_id: i64,
    pub timestamp: String,
    pub age_days: f64,
    pub reason: String,
}

impl Outputable for CleanupPlanOutput {
    fn to_text(&self) -> String {
        let mut lines = vec![
            format!("Snapshot Cleanup Plan for {}", self.table),
            String::new(),
            format!("Policy:"),
            format!("  Older than:  {} days", self.older_than_days),
            format!("  Retain last: {} snapshots", self.retain_last),
            String::new(),
        ];

        if self.snapshots_to_remove.is_empty() {
            lines.push("No snapshots eligible for removal.".to_string());
        } else {
            lines.push(format!(
                "Snapshots to remove ({}):",
                self.snapshots_to_remove.len()
            ));

            let mut table = ComfyTable::new();
            table.set_header(Row::from(vec![
                "Snapshot ID",
                "Timestamp",
                "Age",
                "Operation",
            ]));

            for snapshot in &self.snapshots_to_remove {
                table.add_row(Row::from(vec![
                    snapshot.snapshot_id.to_string(),
                    snapshot.timestamp.clone(),
                    format!("{:.1}d", snapshot.age_days),
                    snapshot.operation.clone(),
                ]));
            }
            lines.push(table.to_string());
        }

        lines.push(String::new());
        lines.push(format!(
            "Snapshots to retain ({}):",
            self.snapshots_to_retain.len()
        ));

        if !self.snapshots_to_retain.is_empty() {
            let mut retain_table = ComfyTable::new();
            retain_table.set_header(Row::from(vec!["Snapshot ID", "Timestamp", "Age", "Reason"]));

            for snapshot in &self.snapshots_to_retain {
                retain_table.add_row(Row::from(vec![
                    snapshot.snapshot_id.to_string(),
                    snapshot.timestamp.clone(),
                    format_age(snapshot.age_days),
                    snapshot.reason.clone(),
                ]));
            }
            lines.push(retain_table.to_string());
        }

        lines.push(String::new());
        lines.push("Summary".to_string());
        lines.push(format!(
            "  Total:   {} snapshots",
            format_number(self.total_snapshots as u64)
        ));
        lines.push(format!(
            "  Remove:  {} snapshots",
            format_number(self.snapshots_to_remove.len() as u64)
        ));
        lines.push(format!(
            "  Retain:  {} snapshots",
            format_number(self.snapshots_to_retain.len() as u64)
        ));

        if self.dry_run && !self.snapshots_to_remove.is_empty() {
            lines.push(String::new());
            lines.push("Dry run complete. Remove --dry-run to execute.".to_string());
        }

        lines.join("\n")
    }
}

/// Cleanup result output
#[derive(Debug, Serialize)]
pub struct CleanupResultOutput {
    pub table: String,
    pub snapshots_removed: usize,
    pub snapshots_retained: usize,
    pub removed_snapshot_ids: Vec<i64>,
}

impl Outputable for CleanupResultOutput {
    fn to_text(&self) -> String {
        let mut lines = vec![format!("Snapshot Cleanup Complete for {}", self.table)];

        lines.push(String::new());
        lines.push(format!(
            "Removed:  {} snapshots",
            format_number(self.snapshots_removed as u64)
        ));
        lines.push(format!(
            "Retained: {} snapshots",
            format_number(self.snapshots_retained as u64)
        ));

        if !self.removed_snapshot_ids.is_empty() && self.removed_snapshot_ids.len() <= 10 {
            lines.push(String::new());
            lines.push("Removed snapshot IDs:".to_string());
            for id in &self.removed_snapshot_ids {
                lines.push(format!("  {}", id));
            }
        }

        lines.join("\n")
    }
}

fn format_timestamp(timestamp_ms: i64) -> String {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Invalid timestamp".to_string())
}

fn format_age(age_days: f64) -> String {
    if age_days < 1.0 {
        format!("{:.1}h", age_days * 24.0)
    } else {
        format!("{:.1}d", age_days)
    }
}

fn format_retention_reason(reason: &RetentionReason) -> String {
    match reason {
        RetentionReason::CurrentSnapshot => "Current snapshot".to_string(),
        RetentionReason::WithinRetainCount => "Within retain-last count".to_string(),
        RetentionReason::NotOldEnough => "Not old enough".to_string(),
        RetentionReason::ReferencedByRef(refs) => format!("Referenced by: {}", refs),
    }
}

fn build_cleanup_plan_output(table: &str, plan: &CleanupPlan, dry_run: bool) -> CleanupPlanOutput {
    let snapshots_to_remove: Vec<SnapshotToRemove> = plan
        .snapshots_to_remove
        .iter()
        .map(|s| SnapshotToRemove {
            snapshot_id: s.snapshot_id,
            timestamp: format_timestamp(s.timestamp_ms),
            age_days: s.age_days,
            operation: s.operation.clone(),
        })
        .collect();

    let snapshots_to_retain: Vec<SnapshotToRetain> = plan
        .snapshots_to_retain
        .iter()
        .map(|s| SnapshotToRetain {
            snapshot_id: s.info.snapshot_id,
            timestamp: format_timestamp(s.info.timestamp_ms),
            age_days: s.info.age_days,
            reason: format_retention_reason(&s.reason),
        })
        .collect();

    CleanupPlanOutput {
        table: table.to_string(),
        older_than_days: plan.older_than_days,
        retain_last: plan.retain_last,
        total_snapshots: plan.total_snapshots,
        snapshots_to_remove,
        snapshots_to_retain,
        dry_run,
    }
}

/// Execute a snapshot command
pub async fn execute(
    command: SnapshotCommand,
    config: &CatalogConfig,
    format: OutputFormat,
) -> Result<(), String> {
    let catalog = config.create_catalog().await?;

    match command {
        SnapshotCommand::List { table: table_str } => {
            let table_ident = parse_table_ident(&table_str)?;
            let table = catalog
                .load_table(&table_ident)
                .await
                .map_err(|e| format!("Failed to load table: {}", e))?;

            let metadata = table.metadata();
            let current_snapshot_id = metadata.current_snapshot_id();

            // Build map of snapshot_id -> ref names
            let mut snapshot_refs: std::collections::HashMap<i64, Vec<String>> =
                std::collections::HashMap::new();
            for (ref_name, snapshot_ref) in metadata.refs() {
                snapshot_refs
                    .entry(snapshot_ref.snapshot_id())
                    .or_default()
                    .push(ref_name.clone());
            }

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("Failed to get current time: {}", e))?
                .as_millis() as i64;

            let mut snapshots: Vec<SnapshotEntry> = metadata
                .snapshots()
                .iter()
                .map(|s| {
                    let age_ms = now_ms - s.timestamp_ms();
                    let age_days = age_ms as f64 / (24.0 * 60.0 * 60.0 * 1000.0);

                    SnapshotEntry {
                        snapshot_id: s.snapshot_id(),
                        timestamp: format_timestamp(s.timestamp_ms()),
                        age_days,
                        operation: s.summary().operation().to_string(),
                        is_current: current_snapshot_id == Some(s.snapshot_id()),
                        refs: snapshot_refs
                            .get(&s.snapshot_id())
                            .cloned()
                            .unwrap_or_default(),
                    }
                })
                .collect();

            // Sort by timestamp descending (newest first)
            snapshots.sort_by(|a, b| b.snapshot_id.cmp(&a.snapshot_id));

            let result = SnapshotList {
                table: table_str,
                total_count: snapshots.len(),
                current_snapshot_id,
                snapshots,
            };

            print(&result, format);
            Ok(())
        }

        SnapshotCommand::Cleanup {
            table: table_str,
            older_than_days,
            retain_last,
            dry_run,
        } => {
            let table_ident = parse_table_ident(&table_str)?;
            let table = catalog
                .load_table(&table_ident)
                .await
                .map_err(|e| format!("Failed to load table: {}", e))?;

            // Build cleanup options
            let options = CleanupOptions::new()
                .with_older_than_days(older_than_days)
                .with_retain_last(retain_last);

            // Create cleanup plan
            let plan = plan_snapshot_cleanup(&table, &options)
                .map_err(|e| format!("Failed to create cleanup plan: {}", e))?;

            if plan.is_empty() {
                println!("No snapshots eligible for cleanup.");
                return Ok(());
            }

            if dry_run {
                // Output plan
                let plan_output = build_cleanup_plan_output(&table_str, &plan, true);
                print(&plan_output, format);
                return Ok(());
            }

            // Show plan first
            let plan_output = build_cleanup_plan_output(&table_str, &plan, false);
            print(&plan_output, format);

            println!("\nExecuting cleanup...");

            // Execute cleanup
            let snapshot_ids: Vec<i64> = plan
                .snapshots_to_remove
                .iter()
                .map(|s| s.snapshot_id)
                .collect();

            catalog
                .expire_snapshots(&table_ident, &snapshot_ids)
                .await
                .map_err(|e| format!("Cleanup failed: {}", e))?;

            let result = CleanupResultOutput {
                table: table_str,
                snapshots_removed: plan.snapshots_to_remove.len(),
                snapshots_retained: plan.snapshots_to_retain.len(),
                removed_snapshot_ids: snapshot_ids,
            };

            print(&result, format);
            Ok(())
        }
    }
}
