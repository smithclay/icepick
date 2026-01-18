//! Types for REST catalog commit operations

use serde::{Deserialize, Serialize};

/// Request to commit table changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitTableRequest {
    pub requirements: Vec<TableRequirement>,
    pub updates: Vec<TableUpdate>,
}

/// Requirements that must be met for commit to succeed
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[allow(clippy::enum_variant_names)]
pub enum TableRequirement {
    #[serde(rename = "assert-table-uuid")]
    AssertTableUuid { uuid: String },

    #[serde(rename = "assert-current-schema-id")]
    AssertCurrentSchemaId {
        #[serde(rename = "current-schema-id")]
        current_schema_id: i32,
    },

    #[serde(rename = "assert-last-assigned-field-id")]
    AssertLastAssignedFieldId {
        #[serde(rename = "last-assigned-field-id")]
        last_assigned_field_id: i32,
    },

    #[serde(rename = "assert-ref-snapshot-id")]
    AssertRefSnapshotId {
        #[serde(rename = "ref")]
        r#ref: String,
        #[serde(rename = "snapshot-id")]
        snapshot_id: Option<i64>,
    },
}

/// Updates to apply to the table
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum TableUpdate {
    #[serde(rename = "set-current-table-metadata")]
    SetCurrentTableMetadata {
        #[serde(rename = "metadata-location")]
        metadata_location: String,
        metadata: Box<crate::spec::TableMetadata>,
    },

    #[serde(rename = "add-snapshot")]
    AddSnapshot { snapshot: crate::spec::Snapshot },

    #[serde(rename = "set-snapshot-ref")]
    SetSnapshotRef {
        #[serde(rename = "ref-name")]
        ref_name: String,
        #[serde(rename = "snapshot-id")]
        snapshot_id: i64,
        #[serde(rename = "type")]
        ref_type: String,
        /// Optional retention policy fields for branches
        #[serde(
            rename = "min-snapshots-to-keep",
            skip_serializing_if = "Option::is_none"
        )]
        min_snapshots_to_keep: Option<i32>,
        #[serde(
            rename = "max-snapshot-age-ms",
            skip_serializing_if = "Option::is_none"
        )]
        max_snapshot_age_ms: Option<i64>,
        #[serde(rename = "max-ref-age-ms", skip_serializing_if = "Option::is_none")]
        max_ref_age_ms: Option<i64>,
    },

    /// Remove snapshots by their IDs
    #[serde(rename = "remove-snapshots")]
    RemoveSnapshots {
        #[serde(rename = "snapshot-ids")]
        snapshot_ids: Vec<i64>,
    },

    #[serde(rename = "upgrade-format-version")]
    UpgradeFormatVersion {
        #[serde(rename = "format-version")]
        format_version: i32,
    },
}

/// Response from commit operation
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct CommitTableResponse {
    #[serde(rename = "metadata-location")]
    pub metadata_location: String,
    pub metadata: crate::spec::TableMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{NestedField, PrimitiveType, Schema, TableMetadata, Type};

    #[test]
    fn test_commit_types_serialize() {
        let schema = Schema::builder()
            .with_fields(vec![NestedField::required_field(
                1,
                "id".to_string(),
                Type::Primitive(PrimitiveType::Long),
            )])
            .build()
            .unwrap();

        let metadata = TableMetadata::builder()
            .with_location("s3://bucket/table")
            .with_current_schema(schema)
            .build()
            .unwrap();

        let req = CommitTableRequest {
            requirements: vec![
                TableRequirement::AssertTableUuid {
                    uuid: metadata.table_uuid().to_string(),
                },
                TableRequirement::AssertRefSnapshotId {
                    r#ref: "main".to_string(),
                    snapshot_id: Some(1),
                },
            ],
            updates: vec![
                TableUpdate::SetCurrentTableMetadata {
                    metadata_location: "s3://bucket/table/metadata/v1.metadata.json".to_string(),
                    metadata: Box::new(metadata.clone()),
                },
                TableUpdate::SetSnapshotRef {
                    ref_name: "main".to_string(),
                    snapshot_id: 2,
                    ref_type: "branch".to_string(),
                    min_snapshots_to_keep: None,
                    max_snapshot_age_ms: None,
                    max_ref_age_ms: None,
                },
            ],
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("assert-ref-snapshot-id"));
        assert!(json.contains("assert-table-uuid"));
        assert!(json.contains("set-snapshot-ref"));
        assert!(json.contains("set-current-table-metadata"));
    }
}
