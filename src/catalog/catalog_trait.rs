//! Core Catalog trait for Iceberg catalogs
//! New design - simpler than iceberg-rust

use async_trait::async_trait;
use std::collections::HashMap;

use crate::error::Result;
use crate::spec::{NamespaceIdent, TableCreation, TableIdent};
use crate::table::Table;

/// Core catalog operations for Iceberg tables
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait Catalog: Send + Sync {
    /// Create a namespace (idempotent - returns Ok if already exists)
    async fn create_namespace(
        &self,
        namespace: &NamespaceIdent,
        properties: HashMap<String, String>,
    ) -> Result<()>;

    /// Check if a namespace exists
    async fn namespace_exists(&self, namespace: &NamespaceIdent) -> Result<bool>;

    /// List all namespaces in the catalog
    async fn list_namespaces(&self) -> Result<Vec<NamespaceIdent>> {
        Err(crate::error::Error::invalid_input(
            "list_namespaces not implemented for this catalog",
        ))
    }

    /// List all tables in a namespace
    async fn list_tables(&self, namespace: &NamespaceIdent) -> Result<Vec<TableIdent>>;

    /// Check if a table exists
    async fn table_exists(&self, identifier: &TableIdent) -> Result<bool>;

    /// Create a new table
    async fn create_table(
        &self,
        namespace: &NamespaceIdent,
        creation: TableCreation,
    ) -> Result<Table>;

    /// Load an existing table
    async fn load_table(&self, identifier: &TableIdent) -> Result<Table>;

    /// Delete a table
    async fn drop_table(&self, identifier: &TableIdent) -> Result<()>;

    /// Update table metadata location atomically
    ///
    /// This method atomically updates the catalog's pointer to the table metadata.
    /// If the current metadata location doesn't match `old_metadata_location`,
    /// returns a ConcurrentModification error.
    ///
    /// # Arguments
    /// * `identifier` - The table identifier
    /// * `old_metadata_location` - Expected current metadata location (for optimistic locking)
    /// * `new_metadata_location` - New metadata location to set
    async fn update_table_metadata(
        &self,
        identifier: &TableIdent,
        old_metadata_location: &str,
        new_metadata_location: &str,
    ) -> Result<()>;

    /// Update table schema (optional, for schema evolution support)
    ///
    /// This method updates the table's schema by writing new metadata with the updated schema
    /// and atomically updating the catalog pointer. The default implementation returns an error
    /// indicating schema evolution is not supported for this catalog type.
    ///
    /// # Arguments
    /// * `identifier` - The table identifier
    /// * `new_schema` - The new schema to apply
    ///
    /// # Returns
    /// The updated Table with the new schema
    async fn update_table_schema(
        &self,
        _identifier: &TableIdent,
        _new_schema: crate::spec::Schema,
    ) -> Result<Table> {
        Err(crate::error::Error::invalid_input(
            "Schema evolution not supported for this catalog implementation",
        ))
    }

    /// Expire (remove) snapshots from a table by their IDs
    ///
    /// This method removes the specified snapshots from the table metadata,
    /// allowing their associated data files to be garbage collected.
    ///
    /// # Arguments
    /// * `identifier` - The table identifier
    /// * `snapshot_ids` - List of snapshot IDs to expire
    ///
    /// # Returns
    /// Ok(()) if successful, error otherwise
    ///
    /// # Notes
    /// - The current snapshot cannot be expired
    /// - Snapshots referenced by branches or tags should not be expired
    async fn expire_snapshots(
        &self,
        _identifier: &TableIdent,
        _snapshot_ids: &[i64],
    ) -> Result<()> {
        Err(crate::error::Error::invalid_input(
            "Snapshot expiration not supported for this catalog implementation",
        ))
    }
}
