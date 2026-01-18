# AGENTS.md - icepick

## EXECUTIVE SUMMARY

**icepick** is an experimental Rust client for Apache Iceberg that provides simple, production-ready access to cloud-native Iceberg catalogs (AWS S3 Tables and Cloudflare R2). Unlike the official iceberg-rust library, icepick targets WASM compilation for serverless environments and focuses on REST catalog implementations with minimal configuration. The library abstracts authentication, catalog REST APIs, and file I/O while exposing a clean, type-safe interface for reading and writing Iceberg tables.

Key capabilities include a CLI for table maintenance operations, bin-pack compaction, partition pruning with predicate pushdown, and vended credential caching for REST catalogs.

## QUICK START

```toml
# Add to Cargo.toml
[dependencies]
icepick = "0.3"
tokio = { version = "1", features = ["full"] }
```

### AWS S3 Tables (native platforms only)

```rust
use icepick::catalog::Catalog;
use icepick::{S3TablesCatalog, spec::{NamespaceIdent, TableIdent}};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create catalog from S3 Tables ARN
    let catalog = S3TablesCatalog::from_arn(
        "my-catalog",
        "arn:aws:s3tables:us-west-2:123456789012:bucket/my-bucket"
    ).await?;

    // Load and read a table
    let table_id = TableIdent::from_strs(&["namespace"], "table_name");
    let table = catalog.load_table(&table_id).await?;

    // Scan table data
    let scan = table.scan().build()?;
    let mut stream = scan.to_arrow().await?;

    Ok(())
}
```

### Cloudflare R2 (WASM-compatible)

```rust
use icepick::{R2Catalog, catalog::Catalog, spec::TableIdent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = R2Catalog::new(
        "my-catalog",
        "account-id",
        "bucket-name",
        "cloudflare-api-token"
    ).await?;

    let table_id = "namespace.table_name".parse()?;
    let table = catalog.load_table(&table_id).await?;

    Ok(())
}
```

### CLI (native only)

```bash
# Install with CLI feature
cargo install icepick --features cli

# Set catalog credentials
export ICEPICK_CATALOG_URL="https://catalog.cloudflarestorage.com/account/bucket"
export ICEPICK_TOKEN="your-api-token"

# List namespaces and tables
icepick namespace list
icepick table list --namespace my_namespace
icepick table info my_namespace.my_table

# Scan with filter (shows pruning stats)
icepick table scan my_namespace.my_table --filter "date >= '2024-01-01'"

# Compact small files (dry run first)
icepick compact my_namespace.my_table --dry-run
icepick compact my_namespace.my_table --target-size 268435456

# Snapshot management
icepick snapshot list my_namespace.my_table
icepick snapshot cleanup my_namespace.my_table --dry-run
icepick snapshot cleanup my_namespace.my_table --older-than-days 7 --retain-last 10
```

## CORE CONCEPTS

- **REST Catalog Pattern**: All catalog operations use REST API calls with platform-specific authentication (SigV4 for AWS, bearer tokens for Cloudflare)
- **WASM Compatibility**: R2Catalog compiles to wasm32-unknown-unknown; S3TablesCatalog requires native AWS SDK and is native-only
- **Optimistic Concurrency**: Transactions use metadata location pointers for atomic commits with automatic retry on concurrent modification
- **FileIO Abstraction**: Built on OpenDAL for cross-platform storage access; supports both single-operator (R2) and multi-bucket dynamic operator creation (S3 Tables)
- **Type-Safe Identifiers**: `TableIdent` and `NamespaceIdent` enforce valid naming with compile-time safety

## API SURFACE

```
Module Structure:
├── catalog/          # Catalog implementations (S3TablesCatalog, R2Catalog)
│   ├── auth/        # Authentication (SigV4, bearer tokens)
│   ├── rest/        # REST catalog protocol with vended credential caching
│   └── register/    # Register existing Parquet files without rewriting
├── cli/             # CLI commands (native only, behind "cli" feature)
│   └── commands/    # catalog, namespace, table, compact subcommands
├── compact/         # Bin-pack compaction for small files
├── snapshot_cleanup/ # Snapshot expiration and cleanup
├── expr/            # Predicate expressions for partition pruning
├── spec/            # Iceberg specification types (Schema, TableIdent, etc.)
├── table/           # Table representation and operations
├── transaction/     # Write operations with ACID guarantees
├── scan/            # Table scanning with predicate-based filtering
├── io/              # FileIO abstraction over OpenDAL
├── writer/          # Parquet writing (both Iceberg and standalone)
├── reader/          # Manifest and data file reading
├── manifest/        # Iceberg manifest handling (Avro)
└── error/           # Structured error types
```

### Most Important Public Items

1. **S3TablesCatalog::from_arn()** - Create AWS S3 Tables catalog
2. **R2Catalog::new()** - Create Cloudflare R2 catalog
3. **Catalog trait** - Core operations (create_table, load_table, list_tables, list_namespaces, drop_table)
4. **Table** - Iceberg table with scan() and transaction() methods
5. **TableScanBuilder::filter()** - Add predicate for partition/bounds pruning
6. **TableScan::to_arrow()** - Read table as Arrow RecordBatch stream
7. **Transaction::append().commit()** - Append data files atomically
8. **compact_table()** - Bin-pack compaction (merge small files into larger ones)
9. **plan_compaction()** - Create compaction plan without executing
10. **parse_filter()** - Parse string filter expression into Predicate
11. **Predicate** - Filter expressions (eq, gt, lt, and, or) for partition pruning
12. **arrow_to_parquet()** - Write Arrow data directly to S3 without Iceberg metadata
13. **register_data_files()** - Register existing Parquet files without rewriting data
14. **introspect_parquet_file()** - Extract schema, row count, and metrics from Parquet footer
15. **plan_snapshot_cleanup()** - Plan which snapshots to expire based on retention policy
16. **execute_snapshot_cleanup()** - Execute a cleanup plan to remove expired snapshots
17. **CleanupOptions** - Configure snapshot retention (older_than_days, retain_last)

## COMMON PATTERNS

### Pattern 1: Creating and writing to a table

```rust
use icepick::catalog::Catalog;
use icepick::spec::{NestedField, PrimitiveType, Schema, TableCreation, Type};

// Build Iceberg schema
let schema = Schema::builder()
    .with_fields(vec![
        NestedField::required_field(1, "id".to_string(),
            Type::Primitive(PrimitiveType::Long)),
        NestedField::optional_field(2, "name".to_string(),
            Type::Primitive(PrimitiveType::String)),
    ])
    .build()?;

// Create table
let table_creation = TableCreation::builder()
    .with_name("my_table")
    .with_schema(schema)
    .build()?;

let table = catalog.create_table(&namespace, table_creation).await?;

// Write data using ParquetWriter
use icepick::writer::ParquetWriter;
let mut writer = ParquetWriter::new(table.schema()?.clone())?;
writer.write_batch(&arrow_batch)?;

let data_file = writer.finish(
    table.file_io(),
    format!("{}/data/{}.parquet", table.location(), uuid::Uuid::new_v4())
).await?;

// Commit transaction
table.transaction()
    .append(vec![data_file])
    .commit(&catalog)
    .await?;
```

### Pattern 2: Reading table data

```rust
use futures::StreamExt;

let table = catalog.load_table(&table_id).await?;

// Option A: Get data file list
let files = table.files().await?;
for file in files {
    println!("{} - {} rows", file.file_path, file.record_count);
}

// Option B: Stream as Arrow batches
let scan = table.scan().build()?;
let mut stream = scan.to_arrow().await?;

while let Some(batch_result) = stream.next().await {
    let batch = batch_result?;
    // Process batch
}
```

### Pattern 3: Error handling

```rust
use icepick::Error;

match catalog.load_table(&table_id).await {
    Ok(table) => { /* use table */ },
    Err(Error::NotFound { resource }) => {
        eprintln!("Table not found: {}", resource);
    },
    Err(Error::ConcurrentModification { message }) => {
        // Retry transaction
    },
    Err(e) => return Err(e.into()),
}
```

### Pattern 4: Direct Parquet writes (without Iceberg metadata)

```rust
use icepick::{arrow_to_parquet, FileIO, io::AwsCredentials};
use parquet::basic::Compression;

let file_io = FileIO::from_aws_credentials(
    AwsCredentials {
        access_key_id: "key".to_string(),
        secret_access_key: "secret".to_string(),
        session_token: None,
    },
    "us-west-2".to_string()
);

// Simple write
arrow_to_parquet(&batch, "s3://bucket/data.parquet", &file_io).await?;

// With compression
arrow_to_parquet(&batch, "s3://bucket/data.parquet", &file_io)
    .with_compression(Compression::ZSTD(Default::default()))
    .await?;
```

### Pattern 5: Register existing Parquet files

```rust
use icepick::{introspect_parquet_file, DataFileRegistrar, RegisterOptions};
use icepick::spec::{NamespaceIdent, TableIdent};

// Introspect file to get metadata (schema, row count, size, partition values)
let introspection = introspect_parquet_file(
    catalog.file_io(),
    "s3://bucket/data/year=2025/file.parquet",
    Some(&partition_spec), // extracts Hive-style partition values from path
).await?;

// Register files - creates table if needed, skips already-committed files
let options = RegisterOptions::new()
    .allow_create_with_schema(introspection.schema.clone())
    .allow_noop(true); // idempotent

let result = catalog.register_data_files(
    namespace,
    table_ident,
    vec![introspection.data_file],
    options,
).await?;

println!("Added {} files, {} records", result.added_files, result.added_records);
```

### Pattern 6: Filtering with partition pruning

```rust
use icepick::expr::{Predicate, Datum, parse_filter};
use futures::StreamExt;

let table = catalog.load_table(&table_id).await?;

// Option A: Build predicate programmatically
let predicate = Predicate::and([
    Predicate::gt_eq("date", Datum::Date(19724)), // 2024-01-01
    Predicate::lt("date", Datum::Date(19755)),    // 2024-02-01
    Predicate::eq("status", "active"),
]);

// Option B: Parse from string (useful for CLI/user input)
let predicate = parse_filter("date >= '2024-01-01' AND status = 'active'")?;

// Build scan with filter
let scan = table.scan()
    .filter(predicate)
    .build()?;

// Check pruning effectiveness
let (filtered, total) = scan.file_count().await?;
println!("Scanning {} of {} files", filtered, total);

// Stream filtered results
let mut stream = scan.to_arrow().await?;
while let Some(batch) = stream.next().await {
    let batch = batch?;
    // Process batch
}
```

### Pattern 7: Table compaction

```rust
use icepick::compact::{compact_table, plan_compaction, CompactOptions};

let table = catalog.load_table(&table_id).await?;

// Configure compaction options
let options = CompactOptions::new()
    .with_target_file_size(256 * 1024 * 1024)?  // 256 MB target
    .with_max_input_file_size(128 * 1024 * 1024)? // Only compact files < 128 MB
    .with_min_files_per_group(3)?;  // Need at least 3 files to compact

// Option A: Dry run - see what would happen
let plan = plan_compaction(&table, &options).await?;
println!("Would compact {} partitions, {} files",
    plan.partition_count(), plan.total_input_files());

// Option B: Execute compaction
let result = compact_table(&table, &catalog, &options).await?;
println!("Compacted {} files into {}", result.files_removed, result.files_added);
```

### Pattern 8: Snapshot cleanup

```rust
use icepick::snapshot_cleanup::{plan_snapshot_cleanup, execute_snapshot_cleanup, CleanupOptions};

let table = catalog.load_table(&table_id).await?;

// Configure cleanup options
let options = CleanupOptions::new()
    .with_older_than_days(7)   // Expire snapshots older than 7 days
    .with_retain_last(10);      // Always keep at least 10 most recent

// Option A: Dry run - see what would be expired
let plan = plan_snapshot_cleanup(&table, &options)?;
println!("Would remove {} of {} snapshots",
    plan.snapshots_to_remove.len(), plan.total_snapshots);

for snapshot in &plan.snapshots_to_remove {
    println!("  Remove: {} ({:.1} days old)", snapshot.snapshot_id, snapshot.age_days);
}

// Option B: Execute cleanup
if !plan.snapshots_to_remove.is_empty() {
    let result = execute_snapshot_cleanup(&table, &catalog, plan).await?;
    println!("Removed {} snapshots", result.snapshots_removed);
}
```

## INTEGRATION POINTS

- **Async Runtime**: tokio (required for examples/tests, not enforced as dependency)
- **Serialization**: serde with JSON for REST API, apache-avro for manifest files
- **Arrow/Parquet**: Uses arrow 56.2.0 and parquet 56.2.0 crates directly
- **Storage Backend**: OpenDAL 0.54 with services-s3 and services-memory features
- **Authentication**:
  - Native: aws-config, aws-sdk-sts, aws-sigv4, reqwest with rustls-tls
  - WASM: reqwest with JSON (no TLS features)
- **Key Feature Flags**: `cli` (enables the icepick binary); platform selection via cfg(target_family = "wasm")
- **Critical Dependencies**: opendal (storage abstraction), async-trait (catalog trait), thiserror (error types), clap (CLI parsing)

## CONSTRAINTS & GOTCHAS

- **MSRV**: Rust 2021 edition (likely 1.70+, not explicitly documented)
- **Platform-specific behavior**:
  - `S3TablesCatalog` unavailable on WASM (requires AWS SDK)
  - `R2Catalog` uses `?Send` async trait on WASM (single-threaded)
  - Some error variants (e.g., `Error::InvalidArn`) only exist on native platforms
- **Performance cliffs**:
  - `arrow_to_parquet()` buffers entire Parquet file in memory before upload
  - Table scans with predicates prune by partition and column stats, but still read full files (no row-level filtering)
  - No connection pooling for REST catalog calls
  - Compaction loads all files in a group into memory (limit with `max_compaction_group_bytes`)
- **Common misuse patterns**:
  - Don't call `table.files()` in a loop - cache the table metadata
  - Don't create new catalog instances per request - reuse them
  - Always reload table after commit to get latest metadata
- **Unsafe code**: None in the library
- **Concurrent modification handling**: Transactions will fail with `ConcurrentModification` error if table metadata changes between load and commit - client must retry with fresh table metadata

## TESTING GUIDANCE

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use icepick::spec::{NamespaceIdent, TableIdent, Schema, NestedField, Type, PrimitiveType};
    use icepick::io::FileIO;
    use opendal::Operator;

    #[tokio::test]
    async fn test_table_operations() {
        // Setup: Create in-memory FileIO for testing
        let op = Operator::via_iter(opendal::Scheme::Memory, []).unwrap();
        let file_io = FileIO::new(op);

        // Create test schema
        let schema = Schema::builder()
            .with_fields(vec![
                NestedField::required_field(1, "id".to_string(),
                    Type::Primitive(PrimitiveType::Long))
            ])
            .build()
            .unwrap();

        // Test operations
        let metadata = TableMetadata::builder()
            .with_location("memory://test/table")
            .with_current_schema(schema)
            .build()
            .unwrap();

        let table_id = TableIdent::from_strs(&["test"], "table");
        let table = Table::new(table_id, metadata, "memory://test/metadata.json".to_string(), file_io);

        assert_eq!(table.location(), "memory://test/table");
    }
}
```

### Testing with real AWS/Cloudflare services

Use environment variables for credentials in integration tests:

```rust
#[tokio::test]
#[ignore] // Only run with --ignored flag
async fn test_s3_tables_integration() {
    dotenvy::dotenv().ok();
    let arn = std::env::var("S3_TABLES_ARN").unwrap();
    let catalog = S3TablesCatalog::from_arn("test", &arn).await.unwrap();
    // ... test operations
}
```

## CONTRIBUTION VECTORS

- **Code style**: Use `cargo fmt` (standard rustfmt.toml expected but not present)
- **Test coverage**: Unit tests for pure functions, integration tests (with `#[ignore]`) for cloud services
- **Benchmark requirements**: None currently - performance testing is manual
- **Documentation standards**:
  - All public items require doc comments with examples
  - Use `//!` for module-level docs
  - Include `# Errors` section for fallible functions
  - Add `# Examples` with `no_run` for async/cloud examples
- **Where to add new functionality**:
  - New catalog implementations: `src/catalog/<provider>.rs` + update `catalog/mod.rs`
  - New Iceberg spec types: `src/spec/<type>.rs`
  - New write operations: Extend `Transaction` in `src/transaction.rs`
  - New read capabilities: Extend `TableScan` in `src/scan.rs`
  - Storage backend changes: `src/io/file_io.rs`

## SEMANTIC VERSIONING CONTRACT

**Breaking changes** (require major version bump):
- Changes to public trait methods (Catalog, async fn signatures)
- Removal of public types or methods
- Changes to Error enum variants (code matching on them will break)
- Modifications to `TableIdent`, `NamespaceIdent`, or other core spec types
- FileIO method signature changes

**Non-breaking changes** (minor version bump):
- New catalog implementations
- New methods on existing types
- New error variants (if using catch-all patterns)
- Performance improvements without API changes
- New optional features

**Patch changes**:
- Bug fixes in existing functionality
- Documentation improvements
- Internal refactoring
- Dependency updates (within semver compatibility)

## FOR AI AGENTS

### Quick Reference

When working with this library:
1. Always use `catalog::Catalog` trait for catalog operations (don't call REST endpoints directly)
2. Platform check: Use `S3TablesCatalog` for native AWS, `R2Catalog` for WASM/Cloudflare
3. Run `cargo clippy -- -D warnings` before suggesting changes
4. For architecture decisions, this is a thin wrapper over Iceberg REST protocol - prioritize simplicity over feature completeness
5. Error pattern: All errors implement Display with context - use `?` operator and let errors propagate
6. Use predicates for scan filtering: `table.scan().filter(predicate).build()?`
7. Compaction is available via `compact_table()` or `plan_compaction()` + `execute_compaction()`
8. Snapshot cleanup via `plan_snapshot_cleanup()` + `execute_snapshot_cleanup()`
9. CLI is behind the `cli` feature flag (native only)

### Key Invariants to Maintain

- **FileIO must never expose raw OpenDAL Operator** - all file operations go through FileIO methods
- **Catalog implementations must use optimistic locking** - always pass old_metadata_location when updating
- **WASM compatibility for R2Catalog** - never use AWS SDK types in R2 code paths

### When Generating Code Using This Library

**Always:**
- Reload table after commit: `let table = catalog.load_table(&table_id).await?;`
- Use `TableIdent::from_strs(&["namespace"], "table")` for simple construction
- Include proper error handling (don't unwrap on I/O operations)
- Use `#[tokio::main]` or equivalent async runtime in examples
- Add field IDs to Iceberg schemas (required for Parquet field mapping)
- Use `parse_filter()` for user-provided filter strings; use `Predicate::*` for programmatic filters
- Call `plan_compaction()` with `dry_run` first to preview changes before `compact_table()`
- Call `plan_snapshot_cleanup()` first to preview before `execute_snapshot_cleanup()`
- Use `CompactOptions::with_*()` and `CleanupOptions::with_*()` builder patterns

**Never:**
- Construct `Table` directly (use catalog methods)
- Reuse `Transaction` after commit (it consumes self)
- Mix S3TablesCatalog with WASM targets
- Assume tables have snapshots (check with `current_snapshot()`)
- Hardcode credentials in examples (use env vars or function parameters)
- Run compaction without checking `plan.is_empty()` first
- Run snapshot cleanup without checking `plan.snapshots_to_remove.is_empty()` first
- Use CLI features in WASM builds (cli module is `#[cfg(not(target_family = "wasm"))]`)

## PERFORMANCE PROFILE

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| `catalog.load_table()` | O(1) | Single REST API call + metadata JSON parse |
| `catalog.list_namespaces()` | O(1) | Single REST API call |
| `table.files()` | O(m) | Reads manifest list + m manifest files (Avro) |
| `table.scan().filter().to_arrow()` | O(k) | Reads k files after partition/bounds pruning (k ≤ n) |
| `scan.file_count()` | O(m) | Count files without reading data (for pruning stats) |
| `transaction.commit()` | O(m) | Write new manifest files + update metadata (atomic CAS) |
| `plan_compaction()` | O(m) | Reads manifests and groups small files |
| `execute_compaction()` | O(g×f) | Reads/writes g groups × f files per group |
| `arrow_to_parquet()` | O(n) | Full buffer in memory before upload |
| `plan_snapshot_cleanup()` | O(s) | Iterates snapshots and refs |
| `execute_snapshot_cleanup()` | O(1) | Single REST API call to update metadata |

Where m = number of manifest files, n = number of data files, k = files after pruning, s = number of snapshots

## COMPARISON MATRIX

| Feature | icepick | iceberg-rust |
|---------|---------|--------------|
| WASM Support | ✅ R2Catalog | ❌ |
| Native AWS | ✅ S3TablesCatalog | ✅ |
| Full Iceberg Spec | ❌ (REST catalogs only) | ✅ (complete) |
| Dependencies | Lightweight | Heavy (full AWS SDK) |
| Maturity | Experimental | Production (Apache) |
| Transaction API | Simplified (append only) | Full (delete, overwrite, etc.) |
| Query Optimization | Partition/bounds pruning | Predicate pushdown, projection |
| Compaction | ✅ Bin-pack | ✅ Multiple strategies |
| Snapshot Cleanup | ✅ Automatic expiration | ✅ expire_snapshots API |
| CLI Tool | ✅ icepick binary | ❌ |

**When to use icepick**: WASM deployment, serverless environments (Cloudflare Workers), simpler API for append-only workloads, R2 Data Catalog support, CLI-based table maintenance

**When to use iceberg-rust**: Full Iceberg feature support, non-REST catalogs (Glue, Hive, etc.), complex query patterns, production-critical workloads
