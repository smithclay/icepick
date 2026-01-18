//! Catalog trait implementation for IcebergRestCatalog
//!
//! This module bridges the Catalog trait with the implementation methods.
//! Uses conditional compilation to handle Send requirements for native vs WASM.

use super::IcebergRestCatalog;
use async_trait::async_trait;

// Single trait implementation that works for both native and WASM
// The only difference is whether the async trait requires Send
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl crate::catalog::Catalog for IcebergRestCatalog {
    async fn create_namespace(
        &self,
        namespace: &crate::spec::NamespaceIdent,
        properties: std::collections::HashMap<String, String>,
    ) -> crate::error::Result<()> {
        self.create_namespace_impl(namespace, properties).await
    }

    async fn namespace_exists(
        &self,
        namespace: &crate::spec::NamespaceIdent,
    ) -> crate::error::Result<bool> {
        self.namespace_exists_impl(namespace).await
    }

    async fn list_namespaces(&self) -> crate::error::Result<Vec<crate::spec::NamespaceIdent>> {
        self.list_namespaces_impl().await
    }

    async fn list_tables(
        &self,
        namespace: &crate::spec::NamespaceIdent,
    ) -> crate::error::Result<Vec<crate::spec::TableIdent>> {
        self.list_tables_impl(namespace).await
    }

    async fn table_exists(&self, table: &crate::spec::TableIdent) -> crate::error::Result<bool> {
        self.table_exists_impl(table).await
    }

    async fn create_table(
        &self,
        namespace: &crate::spec::NamespaceIdent,
        creation: crate::spec::TableCreation,
    ) -> crate::error::Result<crate::table::Table> {
        self.create_table_impl(namespace, creation).await
    }

    async fn load_table(
        &self,
        table: &crate::spec::TableIdent,
    ) -> crate::error::Result<crate::table::Table> {
        // Wrap load_table with retry logic if configured
        let table = table.clone();
        self.with_retry(|| async { self.load_table_impl(&table).await })
            .await
    }

    async fn drop_table(&self, table: &crate::spec::TableIdent) -> crate::error::Result<()> {
        self.drop_table_impl(table).await
    }

    async fn update_table_metadata(
        &self,
        identifier: &crate::spec::TableIdent,
        old_metadata_location: &str,
        new_metadata_location: &str,
    ) -> crate::error::Result<()> {
        self.update_table_metadata_impl(identifier, old_metadata_location, new_metadata_location)
            .await
    }

    async fn expire_snapshots(
        &self,
        identifier: &crate::spec::TableIdent,
        snapshot_ids: &[i64],
    ) -> crate::error::Result<()> {
        self.expire_snapshots_impl(identifier, snapshot_ids).await
    }
}
