//! Snapshot cleanup and expiration for Iceberg tables
//!
//! This module provides functionality to expire old table snapshots based on
//! configurable retention policies. Snapshot expiration helps reduce metadata
//! overhead, improve table operations, and decrease storage costs.
//!
//! # Retention Policy
//!
//! Snapshot expiration uses two parameters:
//! - `older_than_days`: Age threshold in days
//! - `retain_last`: Minimum snapshot count to always retain
//!
//! Both conditions must be met before a snapshot is expired, ensuring you
//! always retain recent snapshots even if they exceed the age threshold.
//!
//! # Example
//!
//! ```no_run
//! use icepick::snapshot_cleanup::{plan_snapshot_cleanup, CleanupOptions};
//! use icepick::catalog::Catalog;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let catalog: icepick::R2Catalog = todo!();
//! # let table_ident: icepick::TableIdent = todo!();
//! let table = catalog.load_table(&table_ident).await?;
//!
//! // Plan cleanup: expire snapshots older than 7 days, keep at least 10
//! let options = CleanupOptions::new()
//!     .with_older_than_days(7)
//!     .with_retain_last(10);
//!
//! let plan = plan_snapshot_cleanup(&table, &options)?;
//!
//! // Preview what would be removed
//! println!("Would remove {} snapshots", plan.snapshots_to_remove.len());
//!
//! // Execute the cleanup
//! // let result = execute_snapshot_cleanup(&table, &catalog, plan).await?;
//! # Ok(())
//! # }
//! ```

use crate::catalog::Catalog;
use crate::error::{Error, Result};
use crate::spec::{Snapshot, TableMetadata};
use crate::table::Table;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Options for snapshot cleanup operations
#[derive(Debug, Clone)]
pub struct CleanupOptions {
    /// Minimum age in days before a snapshot can be expired
    older_than_days: u32,
    /// Minimum number of snapshots to always retain (most recent)
    retain_last: usize,
    /// Whether this is a dry run (no actual changes)
    dry_run: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self {
            older_than_days: 7,
            retain_last: 10,
            dry_run: false,
        }
    }
}

impl CleanupOptions {
    /// Create new cleanup options with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the minimum age in days before a snapshot can be expired
    pub fn with_older_than_days(mut self, days: u32) -> Self {
        self.older_than_days = days;
        self
    }

    /// Set the minimum number of snapshots to always retain
    pub fn with_retain_last(mut self, count: usize) -> Self {
        self.retain_last = count;
        self
    }

    /// Set whether this is a dry run
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Get the older than days threshold
    pub fn older_than_days(&self) -> u32 {
        self.older_than_days
    }

    /// Get the retain last count
    pub fn retain_last(&self) -> usize {
        self.retain_last
    }

    /// Check if this is a dry run
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }
}

/// Information about a snapshot for cleanup planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    /// Snapshot ID
    pub snapshot_id: i64,
    /// Timestamp when snapshot was created (milliseconds since epoch)
    pub timestamp_ms: i64,
    /// Parent snapshot ID if any
    pub parent_snapshot_id: Option<i64>,
    /// Age in days
    pub age_days: f64,
    /// Operation that created this snapshot
    pub operation: String,
    /// Whether this is the current snapshot
    pub is_current: bool,
    /// Branch or tag names referencing this snapshot
    pub refs: Vec<String>,
}

impl SnapshotInfo {
    fn from_snapshot(
        snapshot: &Snapshot,
        current_snapshot_id: Option<i64>,
        refs: Vec<String>,
        now_ms: i64,
    ) -> Self {
        let age_ms = now_ms - snapshot.timestamp_ms();
        let age_days = age_ms as f64 / (24.0 * 60.0 * 60.0 * 1000.0);

        Self {
            snapshot_id: snapshot.snapshot_id(),
            timestamp_ms: snapshot.timestamp_ms(),
            parent_snapshot_id: snapshot.parent_snapshot_id(),
            age_days,
            operation: snapshot.summary().operation().to_string(),
            is_current: current_snapshot_id == Some(snapshot.snapshot_id()),
            refs,
        }
    }
}

/// Reason why a snapshot is retained
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RetentionReason {
    /// Snapshot is the current table state
    CurrentSnapshot,
    /// Snapshot is within the retain_last count
    WithinRetainCount,
    /// Snapshot is not old enough to expire
    NotOldEnough,
    /// Snapshot is referenced by a branch or tag
    ReferencedByRef(String),
}

/// A snapshot that will be retained with reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetainedSnapshot {
    /// Snapshot information
    pub info: SnapshotInfo,
    /// Reason for retention
    pub reason: RetentionReason,
}

/// Plan for snapshot cleanup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPlan {
    /// Snapshots that will be removed
    pub snapshots_to_remove: Vec<SnapshotInfo>,
    /// Snapshots that will be retained with reasons
    pub snapshots_to_retain: Vec<RetainedSnapshot>,
    /// Total snapshot count before cleanup
    pub total_snapshots: usize,
    /// Options used for this plan
    pub older_than_days: u32,
    pub retain_last: usize,
}

impl CleanupPlan {
    /// Check if the plan has any snapshots to remove
    pub fn is_empty(&self) -> bool {
        self.snapshots_to_remove.is_empty()
    }

    /// Get the number of snapshots that will be removed
    pub fn removal_count(&self) -> usize {
        self.snapshots_to_remove.len()
    }

    /// Get the number of snapshots that will be retained
    pub fn retention_count(&self) -> usize {
        self.snapshots_to_retain.len()
    }
}

/// Result of snapshot cleanup execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Number of snapshots removed
    pub snapshots_removed: usize,
    /// Number of snapshots retained
    pub snapshots_retained: usize,
    /// IDs of removed snapshots
    pub removed_snapshot_ids: Vec<i64>,
    /// Manifest list files that can be garbage collected
    pub orphaned_manifest_lists: Vec<String>,
}

/// Plan which snapshots to cleanup based on the retention policy
///
/// This function determines which snapshots can be safely expired without
/// affecting the current table state or any referenced branches/tags.
pub fn plan_snapshot_cleanup(table: &Table, options: &CleanupOptions) -> Result<CleanupPlan> {
    let metadata = table.metadata();
    let snapshots = metadata.snapshots();

    if snapshots.is_empty() {
        return Ok(CleanupPlan {
            snapshots_to_remove: vec![],
            snapshots_to_retain: vec![],
            total_snapshots: 0,
            older_than_days: options.older_than_days,
            retain_last: options.retain_last,
        });
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| Error::unexpected(format!("Failed to get current time: {}", e)))?
        .as_millis() as i64;

    let current_snapshot_id = metadata.current_snapshot_id();
    let age_threshold_ms = (options.older_than_days as i64) * 24 * 60 * 60 * 1000;

    // Build a map of snapshot_id -> ref names
    let mut snapshot_refs: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    for (ref_name, snapshot_ref) in metadata.refs() {
        snapshot_refs
            .entry(snapshot_ref.snapshot_id())
            .or_default()
            .push(ref_name.clone());
    }

    // Convert snapshots to SnapshotInfo and sort by timestamp (newest first)
    let mut snapshot_infos: Vec<SnapshotInfo> = snapshots
        .iter()
        .map(|s| {
            SnapshotInfo::from_snapshot(
                s,
                current_snapshot_id,
                snapshot_refs.get(&s.snapshot_id()).cloned().unwrap_or_default(),
                now_ms,
            )
        })
        .collect();

    // Sort by timestamp descending (newest first)
    snapshot_infos.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));

    let mut snapshots_to_remove = Vec::new();
    let mut snapshots_to_retain = Vec::new();

    // Track which snapshots are in the "retain last N" window
    let retain_last_ids: HashSet<i64> = snapshot_infos
        .iter()
        .take(options.retain_last)
        .map(|s| s.snapshot_id)
        .collect();

    for info in snapshot_infos {
        // Determine if this snapshot should be retained and why
        let retention_reason = if info.is_current {
            Some(RetentionReason::CurrentSnapshot)
        } else if !info.refs.is_empty() {
            Some(RetentionReason::ReferencedByRef(info.refs.join(", ")))
        } else if retain_last_ids.contains(&info.snapshot_id) {
            Some(RetentionReason::WithinRetainCount)
        } else if (now_ms - info.timestamp_ms) < age_threshold_ms {
            Some(RetentionReason::NotOldEnough)
        } else {
            None
        };

        if let Some(reason) = retention_reason {
            snapshots_to_retain.push(RetainedSnapshot { info, reason });
        } else {
            snapshots_to_remove.push(info);
        }
    }

    Ok(CleanupPlan {
        total_snapshots: snapshots.len(),
        snapshots_to_remove,
        snapshots_to_retain,
        older_than_days: options.older_than_days,
        retain_last: options.retain_last,
    })
}

/// Execute the snapshot cleanup plan
///
/// This removes the specified snapshots from the table metadata and commits
/// the changes to the catalog.
pub async fn execute_snapshot_cleanup<C: Catalog>(
    table: &Table,
    catalog: &C,
    plan: CleanupPlan,
) -> Result<CleanupResult> {
    if plan.is_empty() {
        return Ok(CleanupResult {
            snapshots_removed: 0,
            snapshots_retained: plan.snapshots_to_retain.len(),
            removed_snapshot_ids: vec![],
            orphaned_manifest_lists: vec![],
        });
    }

    let snapshot_ids_to_remove: HashSet<i64> = plan
        .snapshots_to_remove
        .iter()
        .map(|s| s.snapshot_id)
        .collect();

    // Get manifest lists that will become orphaned
    let orphaned_manifest_lists: Vec<String> = table
        .metadata()
        .snapshots()
        .iter()
        .filter(|s| snapshot_ids_to_remove.contains(&s.snapshot_id()))
        .map(|s| s.manifest_list().to_string())
        .collect();

    // Build new metadata without the expired snapshots
    let old_metadata = table.metadata();
    let new_snapshots: Vec<Snapshot> = old_metadata
        .snapshots()
        .iter()
        .filter(|s| !snapshot_ids_to_remove.contains(&s.snapshot_id()))
        .cloned()
        .collect();

    // Filter snapshot log as well
    let new_snapshot_log: Vec<crate::spec::SnapshotLogEntry> = old_metadata
        .snapshot_log()
        .iter()
        .filter(|entry| !snapshot_ids_to_remove.contains(&entry.snapshot_id()))
        .cloned()
        .collect();

    // Build the updated metadata
    let mut builder = TableMetadata::builder()
        .with_format_version(old_metadata.format_version())
        .with_table_uuid(old_metadata.table_uuid().to_string())
        .with_location(old_metadata.location())
        .with_last_updated_ms(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| Error::unexpected(format!("Failed to get current time: {}", e)))?
                .as_millis() as i64,
        )
        .with_last_sequence_number(old_metadata.last_sequence_number())
        .with_snapshot_log(new_snapshot_log)
        .with_metadata_log(old_metadata.metadata_log().to_vec())
        .with_partition_specs(old_metadata.partition_specs().to_vec())
        .with_sort_orders(old_metadata.sort_orders().to_vec())
        .with_refs(old_metadata.refs().clone())
        .with_table_features(old_metadata.table_features().to_vec());

    // Add all schemas
    for schema in old_metadata.schemas() {
        builder = builder.with_current_schema(schema.clone());
    }

    // Add all properties
    for (key, value) in old_metadata.properties() {
        builder = builder.with_property(key.clone(), value.clone());
    }

    // Add the retained snapshots
    for snapshot in new_snapshots {
        // We need to check if this is the current snapshot
        if old_metadata.current_snapshot_id() == Some(snapshot.snapshot_id()) {
            builder = builder.with_current_snapshot(snapshot);
        } else {
            // For non-current snapshots, we need a different approach
            // The builder only has with_current_snapshot, so we need to
            // handle this through the metadata directly after build
        }
    }

    // This is a simplified version - in production you'd want to use
    // the REST catalog's remove-snapshots update type directly
    let removed_ids: Vec<i64> = plan
        .snapshots_to_remove
        .iter()
        .map(|s| s.snapshot_id)
        .collect();

    // Commit via catalog
    catalog
        .expire_snapshots(table.identifier(), &removed_ids)
        .await?;

    Ok(CleanupResult {
        snapshots_removed: plan.snapshots_to_remove.len(),
        snapshots_retained: plan.snapshots_to_retain.len(),
        removed_snapshot_ids: removed_ids,
        orphaned_manifest_lists,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Snapshot, Summary};

    fn create_test_snapshot(id: i64, timestamp_ms: i64, parent_id: Option<i64>) -> Snapshot {
        let manifest_list = format!("s3://bucket/metadata/snap-{}.avro", id);
        let mut builder = Snapshot::builder()
            .with_snapshot_id(id)
            .with_timestamp_ms(timestamp_ms)
            .with_manifest_list(&manifest_list)
            .with_summary(Summary::builder().set("operation", "append").build());

        if let Some(parent) = parent_id {
            builder = builder.with_parent_snapshot_id(parent);
        }

        builder.build().unwrap()
    }

    #[test]
    fn test_cleanup_options_defaults() {
        let options = CleanupOptions::new();
        assert_eq!(options.older_than_days(), 7);
        assert_eq!(options.retain_last(), 10);
        assert!(!options.dry_run());
    }

    #[test]
    fn test_cleanup_options_builder() {
        let options = CleanupOptions::new()
            .with_older_than_days(14)
            .with_retain_last(5)
            .with_dry_run(true);

        assert_eq!(options.older_than_days(), 14);
        assert_eq!(options.retain_last(), 5);
        assert!(options.dry_run());
    }

    #[test]
    fn test_snapshot_info_age_calculation() {
        let now_ms = 1700000000000i64; // Some timestamp
        let one_day_ago_ms = now_ms - (24 * 60 * 60 * 1000);

        let snapshot = create_test_snapshot(1, one_day_ago_ms, None);
        let info = SnapshotInfo::from_snapshot(&snapshot, None, vec![], now_ms);

        assert!((info.age_days - 1.0).abs() < 0.01);
        assert!(!info.is_current);
        assert!(info.refs.is_empty());
    }

    #[test]
    fn test_snapshot_info_current_flag() {
        let now_ms = 1700000000000i64;
        let snapshot = create_test_snapshot(42, now_ms - 1000, None);

        let info = SnapshotInfo::from_snapshot(&snapshot, Some(42), vec![], now_ms);
        assert!(info.is_current);

        let info2 = SnapshotInfo::from_snapshot(&snapshot, Some(99), vec![], now_ms);
        assert!(!info2.is_current);
    }
}
