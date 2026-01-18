use super::rest::IcebergRestCatalog;
use super::{map_catalog_error, AuthProvider, Catalog, CatalogError, CatalogOptions, RetryConfig};
use crate::error::{Error, Result};
use crate::io::FileIO;
use crate::spec::{NamespaceIdent, TableCreation, TableIdent};
use crate::table::Table;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

/// Generic Iceberg REST catalog wrapper that can target any compliant REST endpoint.
///
/// This type exposes the shared REST implementation used by [`R2Catalog`] and
/// [`S3TablesCatalog`], but lets callers provide their own authentication logic,
/// endpoint, and `FileIO` configuration. Use this when connecting to Nessie,
/// Glue REST, or custom Iceberg catalog implementations.
#[derive(Debug)]
pub struct RestCatalog {
    inner: IcebergRestCatalog,
}

impl RestCatalog {
    /// Create a new builder using the provided catalog name and base endpoint.
    pub fn builder(name: impl Into<String>, endpoint: impl Into<String>) -> RestCatalogBuilder {
        RestCatalogBuilder::new(name, endpoint)
    }

    /// Convenience constructor for simple use cases that don't need builder customization.
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        auth_provider: impl RestAuthProvider + 'static,
        file_io: FileIO,
    ) -> Result<Self> {
        Self::builder(name, endpoint)
            .with_auth_provider(auth_provider)
            .with_file_io(file_io)
            .build()
    }
}

/// Builder for constructing [`RestCatalog`] instances with custom options.
pub struct RestCatalogBuilder {
    name: String,
    endpoint: String,
    prefix: String,
    options: CatalogOptions,
    file_io: Option<FileIO>,
    auth_provider: Option<Box<dyn RestAuthProvider>>,
}

impl RestCatalogBuilder {
    fn new(name: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: endpoint.into(),
            prefix: String::new(),
            options: CatalogOptions::default(),
            file_io: None,
            auth_provider: None,
        }
    }

    /// Override the namespace/table prefix that is inserted between `/v1/` and the endpoint path.
    ///
    /// For example: `https://example.com/iceberg` with `prefix = "warehouse"` will produce
    /// requests like `https://example.com/iceberg/v1/warehouse/namespaces/...`.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Override the default [`CatalogOptions`], letting you change HTTP timeouts
    /// or use a non-`main` reference/branch.
    pub fn with_options(mut self, options: CatalogOptions) -> Self {
        self.options = options;
        self
    }

    /// Provide the [`FileIO`] implementation that resolves data files referenced by the catalog.
    ///
    /// Most callers will create a single-operator [`FileIO::new`] using an OpenDAL operator,
    /// or configure AWS credentials via [`FileIO::from_aws_credentials`].
    pub fn with_file_io(mut self, file_io: FileIO) -> Self {
        self.file_io = Some(file_io);
        self
    }

    /// Provide a custom authentication provider used to sign every HTTP request.
    pub fn with_auth_provider<A>(mut self, provider: A) -> Self
    where
        A: RestAuthProvider + 'static,
    {
        self.auth_provider = Some(Box::new(provider));
        self
    }

    /// Convenience helper for bearer token authentication.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth_provider = Some(Box::new(crate::catalog::BearerTokenAuthProvider::new(
            token,
        )));
        self
    }

    /// Configure retry behavior for catalog operations.
    ///
    /// This sets application-level retries based on error types (Transient/Permanent/Timeout).
    /// For HTTP-level retries (connection errors, network issues), use [`CatalogOptions::with_http_config`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use icepick::catalog::{RestCatalog, RetryConfig, BackoffStrategy};
    /// use std::time::Duration;
    /// # use icepick::io::FileIO;
    /// # use opendal::Operator;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let file_io = FileIO::new(Operator::via_iter(opendal::Scheme::Memory, [])?);
    /// let retry = RetryConfig::new(
    ///     5,
    ///     BackoffStrategy::Exponential {
    ///         initial_delay: Duration::from_millis(100),
    ///         max_delay: Duration::from_secs(30),
    ///         multiplier: 2.0,
    ///     }
    /// );
    ///
    /// let catalog = RestCatalog::builder("my-catalog", "https://api.example.com")
    ///     .with_bearer_token("my-token")
    ///     .with_retry_config(retry)
    ///     .with_file_io(file_io)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_retry_config(mut self, retry: RetryConfig) -> Self {
        self.options = self.options.with_retry_config(retry);
        self
    }

    /// Set the request timeout for catalog operations.
    ///
    /// This is a convenience method for configuring HTTP-level timeouts.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use icepick::catalog::RestCatalog;
    /// use std::time::Duration;
    /// # use icepick::io::FileIO;
    /// # use opendal::Operator;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let file_io = FileIO::new(Operator::via_iter(opendal::Scheme::Memory, [])?);
    /// let catalog = RestCatalog::builder("my-catalog", "https://api.example.com")
    ///     .with_bearer_token("my-token")
    ///     .with_timeout(Duration::from_secs(60))
    ///     .with_file_io(file_io)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        let http = self.options.http().clone().with_timeout(timeout);
        self.options = self.options.with_http_config(http);
        self
    }

    /// Construct the [`RestCatalog`], validating that all required components are set.
    pub fn build(self) -> Result<RestCatalog> {
        let file_io = self.file_io.ok_or_else(|| {
            Error::invalid_config("RestCatalog requires a FileIO. Call with_file_io first.")
        })?;
        let auth_provider = self.auth_provider.ok_or_else(|| {
            Error::invalid_config("RestCatalog requires an auth provider. Call with_auth_provider.")
        })?;

        let adapter: Box<dyn AuthProvider> = Box::new(ExternalAuthProvider {
            inner: auth_provider,
        });

        let inner = IcebergRestCatalog::from_components(
            self.name,
            self.endpoint,
            self.prefix,
            adapter,
            file_io,
            self.options,
        )
        .map_err(map_catalog_error)?;

        Ok(RestCatalog { inner })
    }
}

/// Trait for supplying custom authentication logic to the REST catalog.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait RestAuthProvider: Send + Sync + std::fmt::Debug {
    /// Sign or otherwise modify a request before it is sent to the REST endpoint.
    async fn sign_request(&self, request: reqwest::Request) -> Result<reqwest::Request>;
}

struct ExternalAuthProvider {
    inner: Box<dyn RestAuthProvider>,
}

impl std::fmt::Debug for ExternalAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalAuthProvider")
            .finish_non_exhaustive()
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AuthProvider for ExternalAuthProvider {
    async fn sign_request(&self, request: reqwest::Request) -> super::Result<reqwest::Request> {
        self.inner
            .sign_request(request)
            .await
            .map_err(map_auth_error)
    }
}

fn map_auth_error(err: Error) -> CatalogError {
    match err {
        Error::Unauthorized { provider } => CatalogError::AuthError(provider),
        Error::NetworkError { source } => CatalogError::Network(source),
        Error::InvalidRequest { message } => CatalogError::InvalidRequest(message),
        Error::ServerError { status, message } => CatalogError::ServerError { status, message },
        _ => CatalogError::AuthError(err.to_string()),
    }
}

// Implement Catalog trait by delegating to inner Rest implementation (native)
#[cfg(not(target_family = "wasm"))]
#[async_trait]
impl Catalog for RestCatalog {
    async fn create_namespace(
        &self,
        namespace: &NamespaceIdent,
        properties: HashMap<String, String>,
    ) -> Result<()> {
        self.inner.create_namespace(namespace, properties).await
    }

    async fn namespace_exists(&self, namespace: &NamespaceIdent) -> Result<bool> {
        self.inner.namespace_exists(namespace).await
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceIdent>> {
        self.inner.list_namespaces().await
    }

    async fn list_tables(&self, namespace: &NamespaceIdent) -> Result<Vec<TableIdent>> {
        self.inner.list_tables(namespace).await
    }

    async fn table_exists(&self, identifier: &TableIdent) -> Result<bool> {
        self.inner.table_exists(identifier).await
    }

    async fn create_table(
        &self,
        namespace: &NamespaceIdent,
        creation: TableCreation,
    ) -> Result<Table> {
        self.inner.create_table(namespace, creation).await
    }

    async fn load_table(&self, identifier: &TableIdent) -> Result<Table> {
        self.inner.load_table(identifier).await
    }

    async fn drop_table(&self, identifier: &TableIdent) -> Result<()> {
        self.inner.drop_table(identifier).await
    }

    async fn update_table_metadata(
        &self,
        identifier: &TableIdent,
        old_metadata_location: &str,
        new_metadata_location: &str,
    ) -> Result<()> {
        self.inner
            .update_table_metadata(identifier, old_metadata_location, new_metadata_location)
            .await
    }

    async fn expire_snapshots(&self, identifier: &TableIdent, snapshot_ids: &[i64]) -> Result<()> {
        self.inner.expire_snapshots(identifier, snapshot_ids).await
    }
}

// Implement Catalog trait for WASM targets without Send requirement.
#[cfg(target_family = "wasm")]
#[async_trait(?Send)]
impl Catalog for RestCatalog {
    async fn create_namespace(
        &self,
        namespace: &NamespaceIdent,
        properties: HashMap<String, String>,
    ) -> Result<()> {
        self.inner.create_namespace(namespace, properties).await
    }

    async fn namespace_exists(&self, namespace: &NamespaceIdent) -> Result<bool> {
        self.inner.namespace_exists(namespace).await
    }

    async fn list_namespaces(&self) -> Result<Vec<NamespaceIdent>> {
        self.inner.list_namespaces().await
    }

    async fn list_tables(&self, namespace: &NamespaceIdent) -> Result<Vec<TableIdent>> {
        self.inner.list_tables(namespace).await
    }

    async fn table_exists(&self, identifier: &TableIdent) -> Result<bool> {
        self.inner.table_exists(identifier).await
    }

    async fn create_table(
        &self,
        namespace: &NamespaceIdent,
        creation: TableCreation,
    ) -> Result<Table> {
        self.inner.create_table(namespace, creation).await
    }

    async fn load_table(&self, identifier: &TableIdent) -> Result<Table> {
        self.inner.load_table(identifier).await
    }

    async fn drop_table(&self, identifier: &TableIdent) -> Result<()> {
        self.inner.drop_table(identifier).await
    }

    async fn update_table_metadata(
        &self,
        identifier: &TableIdent,
        old_metadata_location: &str,
        new_metadata_location: &str,
    ) -> Result<()> {
        self.inner
            .update_table_metadata(identifier, old_metadata_location, new_metadata_location)
            .await
    }

    async fn expire_snapshots(&self, identifier: &TableIdent, snapshot_ids: &[i64]) -> Result<()> {
        self.inner.expire_snapshots(identifier, snapshot_ids).await
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl RestAuthProvider for crate::catalog::BearerTokenAuthProvider {
    async fn sign_request(&self, request: reqwest::Request) -> Result<reqwest::Request> {
        <Self as AuthProvider>::sign_request(self, request)
            .await
            .map_err(map_catalog_error)
    }
}

#[cfg(not(target_family = "wasm"))]
#[async_trait]
impl RestAuthProvider for crate::catalog::SigV4AuthProvider {
    async fn sign_request(&self, request: reqwest::Request) -> Result<reqwest::Request> {
        <Self as AuthProvider>::sign_request(self, request)
            .await
            .map_err(map_catalog_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendal::Operator;

    #[derive(Debug)]
    struct NoopAuth;

    #[cfg_attr(not(target_family = "wasm"), async_trait)]
    #[cfg_attr(target_family = "wasm", async_trait(?Send))]
    impl RestAuthProvider for NoopAuth {
        async fn sign_request(&self, request: reqwest::Request) -> Result<reqwest::Request> {
            Ok(request)
        }
    }

    fn memory_file_io() -> FileIO {
        let operator =
            Operator::via_iter(opendal::Scheme::Memory, []).expect("memory operator should build");
        FileIO::new(operator)
    }

    #[test]
    fn builder_requires_file_io() {
        let err = RestCatalog::builder("test", "https://example.com/iceberg")
            .with_auth_provider(NoopAuth)
            .build()
            .expect_err("missing FileIO should error");

        assert!(matches!(err, Error::InvalidConfig { .. }));
    }

    #[test]
    fn builder_requires_auth() {
        let err = RestCatalog::builder("test", "https://example.com/iceberg")
            .with_file_io(memory_file_io())
            .build()
            .expect_err("missing auth provider should error");

        assert!(matches!(err, Error::InvalidConfig { .. }));
    }

    #[test]
    fn builder_accepts_components() {
        let result = RestCatalog::builder("test", "https://example.com/iceberg")
            .with_prefix("warehouse")
            .with_file_io(memory_file_io())
            .with_auth_provider(NoopAuth)
            .build();

        assert!(result.is_ok());
    }
}
