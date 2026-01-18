# icepick

[![Crates.io](https://img.shields.io/crates/v/icepick.svg)](https://crates.io/crates/icepick)
[![Documentation](https://docs.rs/icepick/badge.svg)](https://docs.rs/icepick)
[![License](https://img.shields.io/crates/l/icepick.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021%2B-blue.svg)](https://www.rust-lang.org)

> **Experimental client for Apache Iceberg in Rust**

**icepick** provides simple access to Apache Iceberg tables in AWS S3 Tables and Cloudflare R2 Data Catalog. Built on the official [iceberg-rust](https://github.com/apache/iceberg-rust) library, icepick handles authentication, REST API details, and platform compatibility so you can focus on working with your data.

---

### Why icepick?

**Why not use [iceberg-rust](https://github.com/apache/iceberg-rust)?** This project targets WASM as a compilation target (not yet supported in `iceberg-rust`) and focuses on "serverless" catalogs that implement a subset of the overall Iceberg specification.

## Features

### Catalog Support
- **AWS S3 Tables** — Full support with SigV4 authentication (native platforms only)
- **Cloudflare R2 Data Catalog** — Full support with bearer token auth (WASM-compatible)
- **Generic REST Catalog** — Build clients for any Iceberg REST endpoint (Nessie, Glue REST, custom)
- **Direct S3 Parquet Writes** — Write Arrow data directly to S3 without Iceberg metadata

### Table Maintenance
- **Bin-pack Compaction** — Merge small files into larger ones for better query performance
- **Snapshot Cleanup** — Automatically expire old snapshots based on retention policies
- **Partition Pruning** — Filter scans by partition values and column statistics

### Developer Experience
- **Clean API** — Simple factory methods, no complex builders
- **Type-safe errors** — Comprehensive error handling with context
- **Zero-config auth** — Uses AWS credential chain and Cloudflare API tokens
- **Production-ready** — Used in real applications with real data

## Platform Support

| Catalog | Linux/macOS/Windows | WASM (browser/Cloudflare Workers) |
|---------|---------------------|-----------------------------------|
| **S3 Tables** | ✅ | ❌ (requires AWS SDK) |
| **R2 Data Catalog** | ✅ | ✅ |
| **No Catalog** (direct parquet to object storage) | ✅ | ✅ |

> **Note:** R2 Data Catalog and direct Parquet writes are fully WASM-compatible, making them suitable for Cloudflare Workers, browser applications, and other WASM environments.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
icepick = "0.3"
```

## Quick Start

### AWS S3 Tables

```rust
use icepick::S3TablesCatalog;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create catalog from S3 Tables ARN
    let catalog = S3TablesCatalog::from_arn(
        "my-catalog",
        "arn:aws:s3tables:us-west-2:123456789012:bucket/my-bucket"
    ).await?;

    // Load a table
    let table = catalog.load_table(
        &"namespace.table_name".parse()?
    ).await?;

    Ok(())
}
```

### Cloudflare R2 Data Catalog

```rust
use icepick::R2Catalog;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create catalog for R2
    let catalog = R2Catalog::new(
        "my-catalog",
        "account-id",
        "bucket-name",
        "api-token"
    ).await?;

    // Load a table
    let table = catalog.load_table(
        &"namespace.table_name".parse()?
    ).await?;

    Ok(())
}
```

### Generic Iceberg REST Catalog

```rust
use icepick::{FileIO, RestCatalog};
use opendal::Operator;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure your FileIO (S3, R2, filesystem, etc.)
    let operator = Operator::via_iter(opendal::Scheme::Memory, [])?;
    let file_io = FileIO::new(operator);

    // Build a catalog for any Iceberg REST endpoint (Nessie, Glue REST, custom services)
    let catalog = RestCatalog::builder("nessie", "https://nessie.example.com/api/iceberg")
        .with_prefix("warehouse")
        .with_file_io(file_io)
        .with_bearer_token(std::env::var("NESSIE_TOKEN")?)
        .build()?;

    let table = catalog.load_table(&"namespace.table".parse()?).await?;
    Ok(())
}
```

## Authentication

### AWS S3 Tables

Uses the **AWS default credential provider chain** in the following order:

1. Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
2. AWS credentials file (`~/.aws/credentials`)
3. IAM instance profile (EC2)
4. ECS task role

> **Important:** Ensure your credentials have S3 Tables permissions.

### Cloudflare R2 Data Catalog

Uses **Cloudflare API tokens**. To set up:

1. Log into the Cloudflare dashboard
2. Navigate to **My Profile** → **API Tokens**
3. Create a token with **R2 read/write permissions**
4. Pass the token when constructing the catalog

## Direct S3 Parquet Writes

Need to write Parquet files directly to S3 for external tools (Spark, DuckDB, etc.) without Iceberg metadata? Use the `arrow_to_parquet` function:

```rust
use icepick::{arrow_to_parquet, FileIO, io::AwsCredentials};
use arrow::array::{Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::basic::Compression;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup FileIO with AWS credentials
    let file_io = FileIO::from_aws_credentials(
        AwsCredentials {
            access_key_id: "your-key".to_string(),
            secret_access_key: "your-secret".to_string(),
            session_token: None,
        },
        "us-west-2".to_string()
    );

    // Create Arrow data
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )?;

    // Simple write with defaults
    arrow_to_parquet(&batch, "s3://my-bucket/output.parquet", &file_io).await?;

    // With compression
    arrow_to_parquet(&batch, "s3://my-bucket/compressed.parquet", &file_io)
        .with_compression(Compression::ZSTD(parquet::basic::ZstdLevel::default()))
        .await?;

    // Manual partitioning (Hive-style or any structure)
    let date = "2025-01-15";
    let path = format!("s3://my-bucket/data/date={}/data.parquet", date);
    arrow_to_parquet(&batch, &path, &file_io).await?;

    Ok(())
}
```

**Note:** This writes standalone Parquet files without Iceberg metadata. For writing to Iceberg tables, use the `Transaction` API instead.

## Registering Existing Parquet Files

Already have Parquet files in object storage? Register them into an Iceberg table without rewriting data:

```rust
use icepick::{R2Catalog, introspect_parquet_file, DataFileRegistrar, RegisterOptions};
use icepick::spec::{NamespaceIdent, TableIdent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = R2Catalog::new("my-catalog", "account-id", "bucket", "token").await?;

    let namespace = NamespaceIdent::new(vec!["my_namespace".to_string()]);
    let table_ident = TableIdent::new(namespace.clone(), "my_table".to_string());

    // Introspect existing Parquet file to get schema, row count, size
    let introspection = introspect_parquet_file(
        catalog.file_io(),
        "s3://bucket/path/to/file.parquet",
        None, // partition spec (optional)
    ).await?;

    // Register the file - creates table if needed
    let options = RegisterOptions::new()
        .allow_create_with_schema(introspection.schema.clone())
        .allow_noop(true); // idempotent - skip already-registered files

    let result = catalog.register_data_files(
        namespace,
        table_ident,
        vec![introspection.data_file],
        options,
    ).await?;

    println!("Registered {} files ({} records)", result.added_files, result.added_records);
    Ok(())
}
```

This is useful for:
- Migrating existing Parquet datasets to Iceberg
- Registering files written by external tools (Spark, DuckDB, etc.)
- "Write to S3, register later" workflows in serverless environments

## Snapshot Cleanup

Automatically expire old snapshots to reduce metadata overhead and storage costs:

```rust
use icepick::{R2Catalog, snapshot_cleanup::{plan_snapshot_cleanup, execute_snapshot_cleanup, CleanupOptions}};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = R2Catalog::new("my-catalog", "account-id", "bucket", "token").await?;
    let table = catalog.load_table(&"namespace.table".parse()?).await?;

    // Configure retention policy
    let options = CleanupOptions::new()
        .with_older_than_days(7)   // Expire snapshots older than 7 days
        .with_retain_last(10);      // Always keep at least 10 most recent

    // Preview what would be removed
    let plan = plan_snapshot_cleanup(&table, &options)?;
    println!("Will remove {} of {} snapshots",
        plan.snapshots_to_remove.len(), plan.total_snapshots);

    // Execute cleanup
    if !plan.snapshots_to_remove.is_empty() {
        let result = execute_snapshot_cleanup(&table, &catalog, plan).await?;
        println!("Removed {} snapshots", result.snapshots_removed);
    }

    Ok(())
}
```

### CLI Usage

```bash
# List all snapshots with age and status
icepick snapshot list my_namespace.my_table

# Preview cleanup (dry run)
icepick snapshot cleanup my_namespace.my_table --dry-run

# Execute cleanup with custom retention
icepick snapshot cleanup my_namespace.my_table \
  --older-than-days 7 \
  --retain-last 10
```

Snapshot cleanup respects:
- **Current snapshot** - Never expired (it's the current table state)
- **Referenced snapshots** - Never expired if referenced by branches or tags
- **Retention count** - Keeps the N most recent regardless of age
- **Age threshold** - Only expires snapshots older than the threshold

## Examples

Explore complete working examples in the [`examples/`](examples/) directory:

| Example | Description | Command |
|---------|-------------|---------|
| [`s3_tables_basic.rs`](examples/s3_tables_basic.rs) | Complete S3 Tables workflow | `cargo run --example s3_tables_basic` |
| [`r2_basic.rs`](examples/r2_basic.rs) | Complete R2 Data Catalog workflow | `cargo run --example r2_basic` |
| [`r2_register.rs`](examples/r2_register.rs) | Register existing Parquet files | `cargo run --example r2_register` |

## Development

### Running Tests

```bash
cargo test
```

### WASM Build

Verify R2Catalog compiles for WASM:

```bash
cargo build --target wasm32-unknown-unknown
```

### Code Quality

```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Check documentation
cargo doc --no-deps --all-features
```

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Acknowledgments

Built on the official [iceberg-rust](https://github.com/apache/iceberg-rust) library from the Apache Iceberg project.
