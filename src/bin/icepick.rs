//! icepick CLI - Iceberg table maintenance tool

use clap::{Parser, Subcommand};
use icepick::cli::commands::{
    catalog as catalog_cmd, compact as compact_cmd, namespace as namespace_cmd,
    snapshot as snapshot_cmd, table as table_cmd,
};
use icepick::cli::{CatalogConfig, OutputFormat};

/// Iceberg table maintenance CLI
#[derive(Debug, Parser)]
#[command(name = "icepick", about = "Iceberg table maintenance CLI")]
#[command(version, author)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Iceberg REST catalog URL (e.g., https://catalog.cloudflarestorage.com/account/bucket)
    #[arg(long, env = "ICEPICK_CATALOG_URL", global = true)]
    catalog_url: Option<String>,

    /// API Token for catalog authentication
    #[arg(long, env = "ICEPICK_TOKEN", global = true)]
    token: Option<String>,

    /// Output format
    #[arg(long, short, default_value = "text", global = true)]
    output: OutputFormat,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Catalog operations
    #[command(subcommand)]
    Catalog(catalog_cmd::CatalogCommand),

    /// Namespace operations
    #[command(subcommand)]
    Namespace(namespace_cmd::NamespaceCommand),

    /// Table operations
    #[command(subcommand)]
    Table(table_cmd::TableCommand),

    /// Snapshot operations (list, cleanup)
    #[command(subcommand)]
    Snapshot(snapshot_cmd::SnapshotCommand),

    /// Compact a table
    Compact(compact_cmd::CompactArgs),
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();

    let config = CatalogConfig {
        catalog_url: cli.catalog_url,
        token: cli.token,
    };

    let result = match cli.command {
        Commands::Catalog(cmd) => catalog_cmd::execute(cmd, &config, cli.output).await,
        Commands::Namespace(cmd) => namespace_cmd::execute(cmd, &config, cli.output).await,
        Commands::Table(cmd) => table_cmd::execute(cmd, &config, cli.output).await,
        Commands::Snapshot(cmd) => snapshot_cmd::execute(cmd, &config, cli.output).await,
        Commands::Compact(args) => compact_cmd::execute(args, &config, cli.output).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
