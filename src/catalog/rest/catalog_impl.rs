//! Shared implementation methods for IcebergRestCatalog
//!
//! This module contains the actual implementation logic that is shared
//! between native and WASM platforms. The trait implementation itself
//! is in catalog_trait.rs.

use super::commit_types::{CommitTableRequest, TableRequirement, TableUpdate};
use super::helpers;
use super::types::*;
use super::IcebergRestCatalog;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use tracing::warn;

// Private helper functions containing the actual implementation
// These are called by the trait implementation in catalog_trait.rs

impl IcebergRestCatalog {
    pub(super) async fn create_namespace_impl(
        &self,
        namespace: &crate::spec::NamespaceIdent,
        properties: std::collections::HashMap<String, String>,
    ) -> crate::error::Result<()> {
        let namespace_name = namespace.to_string();
        let url = self.url("namespaces");

        let body = CreateNamespaceRequest {
            namespace: vec![namespace_name],
            properties,
        };

        let req = self.build_request(
            self.http_client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body),
        )?;

        let _ = self.execute_request(req).await?;

        Ok(())
    }

    pub(super) async fn namespace_exists_impl(
        &self,
        namespace: &crate::spec::NamespaceIdent,
    ) -> crate::error::Result<bool> {
        let namespace_name = namespace.to_string();
        let url = self.url(&format!("namespaces/{}", namespace_name));

        let req = self.build_request(
            self.http_client
                .get(&url)
                .header("Accept", "application/json"),
        )?;

        let response = self
            .send_request(req)
            .await
            .map_err(helpers::from_catalog_error)?;

        if response.status() == StatusCode::NOT_FOUND {
            // Drain body to allow connection reuse
            let _ = response.bytes().await;
            return Ok(false);
        }

        self.handle_response(response)
            .await
            .map_err(helpers::from_catalog_error)?;

        Ok(true)
    }

    pub(super) async fn list_namespaces_impl(
        &self,
    ) -> crate::error::Result<Vec<crate::spec::NamespaceIdent>> {
        let url = self.url("namespaces");

        let req = self.build_request(
            self.http_client
                .get(&url)
                .header("Accept", "application/json"),
        )?;

        let response: ListNamespacesResponse =
            self.execute_and_parse(req, "namespaces response").await?;

        Ok(response
            .namespaces
            .into_iter()
            .map(crate::spec::NamespaceIdent::new)
            .collect())
    }

    pub(super) async fn list_tables_impl(
        &self,
        namespace: &crate::spec::NamespaceIdent,
    ) -> crate::error::Result<Vec<crate::spec::TableIdent>> {
        let namespace_name = namespace.to_string();
        let url = self.url(&format!("namespaces/{}/tables", namespace_name));

        let req = self.build_request(
            self.http_client
                .get(&url)
                .header("Accept", "application/json"),
        )?;

        let tables: ListTablesResponse = self.execute_and_parse(req, "tables response").await?;

        let mut table_idents = Vec::with_capacity(tables.identifiers.len());
        for ident in tables.identifiers {
            let namespace_ident = crate::spec::NamespaceIdent::new(ident.namespace);
            table_idents.push(crate::spec::TableIdent::new(namespace_ident, ident.name));
        }

        Ok(table_idents)
    }

    pub(super) async fn table_exists_impl(
        &self,
        table: &crate::spec::TableIdent,
    ) -> crate::error::Result<bool> {
        match self.load_table_impl(table).await {
            Ok(_) => Ok(true),
            Err(crate::error::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub(super) async fn create_table_impl(
        &self,
        namespace: &crate::spec::NamespaceIdent,
        creation: crate::spec::TableCreation,
    ) -> crate::error::Result<crate::table::Table> {
        let namespace_name = namespace.to_string();
        let url = self.url(&format!("namespaces/{}/tables", namespace_name));

        let partition_spec = creation
            .partition_spec()
            .map(serde_json::to_value)
            .transpose()?;

        let body = CreateTableRequest {
            name: creation.name().to_string(),
            schema: creation.schema().clone(),
            location: creation.location().map(String::from),
            partition_spec,
            write_order: None, // Will use server defaults
            properties: if creation.properties().is_empty() {
                None
            } else {
                Some(creation.properties().clone())
            },
            stage_create: Some(false),
        };

        let req = self.build_request(
            self.http_client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body),
        )?;

        let table_response: CreateTableResponse =
            self.execute_and_parse(req, "table response").await?;

        // Register the table's identity with the FileIO for credential lookup.
        // This is essential for R2 Data Catalog which uses UUID-based paths
        // that cannot be parsed to extract namespace/table name.
        let table_location = table_response.metadata.location();
        self.file_io
            .register_table(table_location, &namespace_name, creation.name())?;

        let table_ident =
            crate::spec::TableIdent::new(namespace.clone(), creation.name().to_string());
        helpers::build_table(
            table_ident,
            table_response.metadata,
            table_response.metadata_location,
            self.file_io.clone(),
        )
    }

    pub(super) async fn load_table_impl(
        &self,
        table: &crate::spec::TableIdent,
    ) -> crate::error::Result<crate::table::Table> {
        let namespace_name = table.namespace().to_string();
        let url = self.table_url(&namespace_name, table.name(), true);

        let req = self.build_request(
            self.http_client
                .get(&url)
                .header("Accept", "application/json"),
        )?;

        let table_response: LoadTableResponse =
            self.execute_and_parse(req, "table response").await?;

        // Register the table's identity with the FileIO for credential lookup.
        // This is essential for R2 Data Catalog which uses UUID-based paths
        // that cannot be parsed to extract namespace/table name.
        let table_location = table_response.metadata.location();
        self.file_io
            .register_table(table_location, &namespace_name, table.name())?;

        helpers::build_table(
            table.clone(),
            table_response.metadata,
            table_response.metadata_location,
            self.file_io.clone(),
        )
    }

    pub(super) async fn drop_table_impl(
        &self,
        table: &crate::spec::TableIdent,
    ) -> crate::error::Result<()> {
        let namespace_name = table.namespace().to_string();
        let url = self.url(&format!(
            "namespaces/{}/tables/{}?purgeRequested=true",
            namespace_name,
            table.name()
        ));

        let req = self.build_request(
            self.http_client
                .delete(&url)
                .header("Accept", "application/json"),
        )?;

        let _ = self.execute_request(req).await?;

        Ok(())
    }

    /// Helper function to read and optionally decompress metadata files
    fn read_metadata_bytes(path: &str, bytes: Vec<u8>) -> crate::error::Result<Vec<u8>> {
        // Check if the file is gzipped (R2 uses .gz.metadata.json)
        if path.ends_with(".gz.metadata.json") || path.ends_with(".metadata.json.gz") {
            use std::io::Read;
            let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                crate::error::Error::io_error(format!(
                    "Failed to decompress gzipped metadata: {}",
                    e
                ))
            })?;
            Ok(decompressed)
        } else {
            Ok(bytes)
        }
    }

    pub(super) async fn update_table_metadata_impl(
        &self,
        identifier: &crate::spec::TableIdent,
        old_metadata_location: &str,
        new_metadata_location: &str,
    ) -> crate::error::Result<()> {
        // 1. Load current metadata to get current snapshot ID
        let current_metadata_bytes =
            self.file_io
                .read(old_metadata_location)
                .await
                .map_err(|e| {
                    crate::error::Error::io_error(format!("Failed to read old metadata: {}", e))
                })?;

        let current_metadata_bytes =
            Self::read_metadata_bytes(old_metadata_location, current_metadata_bytes)?;

        let current_metadata: crate::spec::TableMetadata =
            serde_json::from_slice(&current_metadata_bytes).map_err(|e| {
                crate::error::Error::invalid_input(format!(
                    "Failed to parse current metadata: {}",
                    e
                ))
            })?;
        let current_snapshot_id = current_metadata.current_snapshot_id();

        // 2. Load new metadata so we can send it to catalog
        let new_metadata_bytes = self
            .file_io
            .read(new_metadata_location)
            .await
            .map_err(|e| {
                crate::error::Error::io_error(format!("Failed to read new metadata: {}", e))
            })?;

        let new_metadata_bytes =
            Self::read_metadata_bytes(new_metadata_location, new_metadata_bytes)?;

        let new_metadata: crate::spec::TableMetadata = serde_json::from_slice(&new_metadata_bytes)
            .map_err(|e| {
                crate::error::Error::invalid_input(format!("Failed to parse new metadata: {}", e))
            })?;

        // 3. Build commit request that performs a CAS on metadata location
        // Note: -1 means "no snapshot", which should be represented as null in REST API
        // Use assert-ref-snapshot-id with the "main" branch reference
        let snapshot_id_requirement = if current_snapshot_id == Some(-1) {
            None
        } else {
            current_snapshot_id
        };

        let reference = self.options.reference().to_string();
        let mut requirements = vec![TableRequirement::AssertTableUuid {
            uuid: current_metadata.table_uuid().to_string(),
        }];
        requirements.push(TableRequirement::AssertRefSnapshotId {
            r#ref: reference.clone(),
            snapshot_id: snapshot_id_requirement,
        });

        let request = CommitTableRequest {
            requirements,
            updates: vec![TableUpdate::SetCurrentTableMetadata {
                metadata_location: new_metadata_location.to_string(),
                metadata: Box::new(new_metadata.clone()),
            }],
        };

        // We use snapshot commit because it's needed for S3 Tables and R2 Catalog
        // as of November 2025
        if !commit_table_enabled() {
            return self
                .legacy_snapshot_commit(identifier, snapshot_id_requirement, new_metadata)
                .await;
        }

        // This is the "preferred API" but not well-supported by R2 Catalog, S3 Tables or Nessie
        match self.commit_table(identifier, request).await {
            Ok(_) => Ok(()),
            Err(err) => {
                // Check if the error indicates unsupported set-current-table-metadata
                let is_unsupported = match &err {
                    crate::catalog::CatalogError::InvalidRequest(ref msg)
                    | crate::catalog::CatalogError::Unexpected(ref msg) => {
                        contains_metadata_error(msg)
                    }
                    crate::catalog::CatalogError::ServerError { message, .. } => {
                        contains_metadata_error(message)
                    }
                    _ => false,
                };

                if is_unsupported {
                    warn!(
                        "Catalog {} does not support set-current-table-metadata, falling back to legacy snapshot updates",
                        self.name
                    );
                    self.legacy_snapshot_commit(identifier, snapshot_id_requirement, new_metadata)
                        .await
                } else {
                    Err(helpers::from_catalog_error(err))
                }
            }
        }
    }

    async fn legacy_snapshot_commit(
        &self,
        identifier: &crate::spec::TableIdent,
        snapshot_requirement: Option<i64>,
        new_metadata: crate::spec::TableMetadata,
    ) -> crate::error::Result<()> {
        let new_snapshot = new_metadata.snapshots().last().cloned().ok_or_else(|| {
            crate::error::Error::invalid_input(
                "New metadata has no snapshots for legacy commit".to_string(),
            )
        })?;
        let new_snapshot_id = new_snapshot.snapshot_id();
        let reference = self.options.reference().to_string();
        let mut requirements = vec![TableRequirement::AssertTableUuid {
            uuid: new_metadata.table_uuid().to_string(),
        }];
        requirements.push(TableRequirement::AssertRefSnapshotId {
            r#ref: reference.clone(),
            snapshot_id: snapshot_requirement,
        });

        let request = CommitTableRequest {
            requirements,
            updates: vec![
                TableUpdate::AddSnapshot {
                    snapshot: new_snapshot,
                },
                TableUpdate::SetSnapshotRef {
                    ref_name: reference,
                    snapshot_id: new_snapshot_id,
                    ref_type: "branch".to_string(),
                    min_snapshots_to_keep: None,
                    max_snapshot_age_ms: None,
                    max_ref_age_ms: None,
                },
            ],
        };

        self.commit_table(identifier, request)
            .await
            .map_err(helpers::from_catalog_error)?;
        Ok(())
    }

    fn build_request(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> crate::error::Result<reqwest::Request> {
        builder
            .build()
            .map_err(|e| crate::error::Error::io_error(format!("Failed to build request: {}", e)))
    }

    async fn execute_request(
        &self,
        req: reqwest::Request,
    ) -> crate::error::Result<serde_json::Value> {
        let response = self
            .send_request(req)
            .await
            .map_err(helpers::from_catalog_error)?;
        self.handle_response(response)
            .await
            .map_err(helpers::from_catalog_error)
    }

    async fn execute_and_parse<T: DeserializeOwned>(
        &self,
        req: reqwest::Request,
        context: &str,
    ) -> crate::error::Result<T> {
        let json_value = self.execute_request(req).await?;
        serde_json::from_value(json_value).map_err(|e| {
            crate::error::Error::invalid_input(format!("Failed to parse {context}: {}", e))
        })
    }
}

fn contains_metadata_error(message: &str) -> bool {
    message.contains("unsupported_table_update")
        || message.contains("unknown variant `set-current-table-metadata`")
        || message.contains("type id 'set-current-table-metadata'")
        || message.contains("set-current-table-metadata")
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
const COMMIT_TABLE_ENV: &str = "ICEPICK_USE_COMMIT_TABLE";

#[cfg(target_family = "wasm")]
fn commit_table_enabled() -> bool {
    false
}

#[cfg(not(target_family = "wasm"))]
fn commit_table_enabled() -> bool {
    match std::env::var(COMMIT_TABLE_ENV) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(
                normalized.as_str(),
                "1" | "true" | "yes" | "on" | "enable" | "enabled"
            )
        }
        Err(_) => false,
    }
}

impl IcebergRestCatalog {
    /// Expire snapshots by their IDs using the REST catalog API
    pub(super) async fn expire_snapshots_impl(
        &self,
        identifier: &crate::spec::TableIdent,
        snapshot_ids: &[i64],
    ) -> crate::error::Result<()> {
        if snapshot_ids.is_empty() {
            return Ok(());
        }

        // Load current table to get requirements
        let table = self.load_table_impl(identifier).await?;
        let metadata = table.metadata();
        let current_snapshot_id = metadata.current_snapshot_id();
        let reference = self.options.reference().to_string();

        // Build commit request with RemoveSnapshots update
        let requirements = vec![
            TableRequirement::AssertTableUuid {
                uuid: metadata.table_uuid().to_string(),
            },
            TableRequirement::AssertRefSnapshotId {
                r#ref: reference,
                snapshot_id: current_snapshot_id,
            },
        ];

        let request = CommitTableRequest {
            requirements,
            updates: vec![TableUpdate::RemoveSnapshots {
                snapshot_ids: snapshot_ids.to_vec(),
            }],
        };

        self.commit_table(identifier, request)
            .await
            .map_err(helpers::from_catalog_error)?;

        Ok(())
    }
}
