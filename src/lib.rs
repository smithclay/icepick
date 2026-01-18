//! Icepick: Production-ready cloud Iceberg catalogs
//!
//! Specialized Rust library for Apache Iceberg catalog operations on cloud providers.
//! Provides rock-solid implementations for AWS S3 Tables and Cloudflare R2 Data Catalog.
//!
//! # Quick Start
//!
//! ## AWS S3 Tables (native platforms only)
//!
//! ```no_run
//! use icepick::S3TablesCatalog;
//! use icepick::catalog::Catalog;
//! use icepick::spec::{TableIdent, NamespaceIdent};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = S3TablesCatalog::from_arn(
//!     "my-catalog",
//!     "arn:aws:s3tables:us-west-2:123456789012:bucket/my-bucket"
//! ).await?;
//!
//! let namespace = NamespaceIdent::new(vec!["namespace".to_string()]);
//! let table_id = TableIdent::new(namespace, "table".to_string());
//! let table = catalog.load_table(&table_id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Cloudflare R2 (all platforms including WASM)
//!
//! ```no_run
//! use icepick::R2Catalog;
//! use icepick::catalog::Catalog;
//! use icepick::spec::{TableIdent, NamespaceIdent};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = R2Catalog::new(
//!     "my-catalog",
//!     "account-id",
//!     "bucket-name",
//!     "api-token"
//! ).await?;
//!
//! let namespace = NamespaceIdent::new(vec!["namespace".to_string()]);
//! let table_id = TableIdent::new(namespace, "table".to_string());
//! let table = catalog.load_table(&table_id).await?;
//! # Ok(())
//! # }
//! ```

pub mod arrow_convert;
pub mod catalog;
#[cfg(not(target_family = "wasm"))]
pub mod cli;
pub mod commit;
pub mod compact;
pub mod error;
pub mod expr;
pub mod io;
pub mod manifest;
pub mod reader;
pub mod scan;
pub mod snapshot_cleanup;
pub mod spec;
pub mod table;
pub mod transaction;
pub mod writer;

// Re-export common types
pub use error::{Error, Result};
pub use io::FileIO;
pub use reader::DataFileEntry;
pub use scan::{ArrowRecordBatchStream, TableScan, TableScanBuilder};
pub use spec::{
    DataContentType, DataFile, NamespaceIdent, NestedField, PrimitiveType, Schema, Snapshot,
    StructType, Summary, TableIdent, TableMetadata, Type,
};
pub use table::Table;
pub use transaction::Transaction;
pub use writer::{
    arrow_to_parquet, AppendOnlyTableWriter, AppendResult, PartitionFieldConfig,
    PartitionTransform, SchemaEvolutionPolicy, TableWriterOptions,
};

// Re-export catalog types
pub use catalog::r2::R2Catalog;
pub use catalog::register::{
    introspect_parquet_file, register_data_files, DataFileFormat, DataFileInput, DataFileRegistrar,
    EncryptionMetadata, FileMetrics, ParquetIntrospection, PartitionValue, RegisterOptions,
    RegisterResult, SkippedFile, SkippedReason,
};
pub use catalog::{RestAuthProvider, RestCatalog, RestCatalogBuilder};

#[cfg(not(target_family = "wasm"))]
pub use catalog::s3_tables::S3TablesCatalog;

// Re-export compaction types
pub use compact::{
    compact_table, execute_compaction, plan_compaction, CompactOptions, CompactionGroup,
    CompactionPlan, CompactionResult, PartitionError, PartitionPlan,
};

// Re-export expression types
pub use expr::{parse_filter, ColumnRef, ComparisonOp, Datum, Predicate};

// Re-export snapshot cleanup types
pub use snapshot_cleanup::{
    execute_snapshot_cleanup, plan_snapshot_cleanup, CleanupOptions, CleanupPlan, CleanupResult,
    RetainedSnapshot, RetentionReason, SnapshotInfo,
};
