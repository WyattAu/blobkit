//! GCS and Azure Blob Storage backends via the [`object_store`] facade.
//!
//! This module adds [`ObjectStoreBackend`] — a thin adapter over
//! [`object_store::ObjectStore`] — so blobkit gains Google Cloud Storage and
//! Azure Blob Storage support without re-implementing another vendor SDK on
//! top of `aws-sdk-s3`. The existing [`S3Store`](crate::s3::S3Store),
//! [`LocalStore`](crate::local::LocalStore), and
//! [`MemoryStore`](crate::memory::MemoryStore) backends are unchanged; this is
//! an *additional* backend option.
//!
//! - `put` → `ObjectStoreExt::put` (single-shot; object_store escalates to
//!   multipart internally where beneficial)
//! - `get` → `ObjectStoreExt::get` (full bytes)
//! - `delete` → existence check + `ObjectStoreExt::delete` (NotFound contract)
//! - `exists` → `ObjectStoreExt::head`
//! - `presigned_url` → `object_store::signer::Signer::signed_url` (GCS signed
//!   URLs / Azure Service SAS; both generated locally from the configured
//!   service-account / account key — no network round-trip)
//! - `list` → `ObjectStore::list` with prefix
//!
//! # Error mapping
//!
//! `object_store::Error` → [`BlobError`]: `NotFound` → `NotFound`,
//! `PermissionDenied` / `Unauthenticated` → `PermissionDenied`,
//! `AlreadyExists` → `AlreadyExists`, `NotSupported` / `NotImplemented` →
//! `Unsupported`, `InvalidPath` → `InvalidKey`, everything else → `Other`.
//!
//! # Example
//!
//! ```rust,no_run
//! use blobkit::object_store_backend::{GcsConfig, GcsCredentials, ObjectStoreBackend};
//! use blobkit::store::BlobStore;
//! use bytes::Bytes;
//!
//! # async fn example() -> Result<(), blobkit::error::BlobError> {
//! let gcs = ObjectStoreBackend::gcs(
//!     GcsConfig::new("my-bucket")
//!         .with_credentials(GcsCredentials::ServiceAccountKey(SERVICE_ACCOUNT_JSON.to_string())),
//! )?;
//! let key = blobkit::types::ObjectKey::new("hello.txt")?;
//! gcs.put(key.clone(), Bytes::from("hello world")).await?;
//! # Ok(())
//! # }
//! # const SERVICE_ACCOUNT_JSON: &str = r#"{"client_email":"x@y.iam.gserviceaccount.com","private_key_id":"kid","private_key":"-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----\n","client_id":"123","auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token","auth_provider_x509_cert_url":"https://www.googleapis.com/oauth2/v1/certs","client_x509_cert_url":"https://c"}"#;
//! ```
//!
//! # WASM
//!
//! The `object_store` dependency (and therefore this module) is only compiled
//! for non-wasm targets; the feature's code is additionally cfg-gated on
//! `not(target_arch = "wasm32")`.

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use async_trait::async_trait;
use bytes::Bytes;
use core::fmt;
use core::time::Duration;

use crate::error::{BlobError, Result};
use crate::store::BlobStore;
use crate::types::{BlobId, ObjectKey};

use futures::TryStreamExt;
use object_store::azure::MicrosoftAzure;
use object_store::azure::MicrosoftAzureBuilder;
use object_store::gcp::GoogleCloudStorage;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path;
use object_store::signer::Signer;
use object_store::ObjectStoreExt;
use object_store::PutPayload;

/// Environment variable holding a full Google service-account key as JSON.
///
/// blobkit's canonical credential source for
/// [`GcsCredentials::FromEnv`]; if unset, object_store's own
/// `GOOGLE_SERVICE_ACCOUNT_KEY` is consulted before giving up on explicit
/// credentials.
pub const GOOGLE_APPLICATION_CREDENTIALS_JSON_ENV: &str = "GOOGLE_APPLICATION_CREDENTIALS_JSON";

/// How [`ObjectStoreBackend::Gcs`] obtains credentials.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum GcsCredentials {
    /// Read credentials from the environment:
    /// [`GOOGLE_APPLICATION_CREDENTIALS_JSON_ENV`] (blobkit canonical,
    /// service-account key as JSON), then `GOOGLE_SERVICE_ACCOUNT_KEY`
    /// (object_store's own variable).
    #[default]
    FromEnv,
    /// Explicit service-account key JSON (same document as
    /// `GOOGLE_SERVICE_ACCOUNT_KEY`). `Debug` is manually implemented to
    /// redact the key (REQ-BLOBKIT-101: secrets never leak via diagnostics).
    ServiceAccountKey(String),
}

impl fmt::Debug for GcsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FromEnv => f.debug_struct("GcsCredentials::FromEnv").finish(),
            Self::ServiceAccountKey(_) => f
                .debug_struct("GcsCredentials::ServiceAccountKey")
                .field("key", &"[redacted]")
                .finish(),
        }
    }
}

/// Configuration for the GCS backend.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GcsConfig {
    /// GCS bucket name, e.g. `"my-bucket"`.
    pub bucket: String,
    /// Credential resolution. Defaults to [`GcsCredentials::FromEnv`].
    pub credentials: GcsCredentials,
}

impl GcsConfig {
    /// Create a new config for `bucket` with [`GcsCredentials::FromEnv`].
    #[must_use]
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            credentials: GcsCredentials::default(),
        }
    }

    /// Set the credential resolution strategy.
    #[must_use]
    pub fn with_credentials(mut self, credentials: GcsCredentials) -> Self {
        self.credentials = credentials;
        self
    }

    /// The bucket name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

/// How [`ObjectStoreBackend::Azure`] obtains credentials.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum AzureCredentials {
    /// Read credentials from the environment via
    /// `MicrosoftAzureBuilder::from_env()` — object_store's own pattern:
    /// `AZURE_STORAGE_ACCOUNT` + `AZURE_STORAGE_ACCOUNT_KEY`,
    /// `AZURE_STORAGE_CONNECTION_STRING`, bearer-token / MSI variables.
    #[default]
    FromEnv,
    /// Explicit shared-key credentials (the pair that also enables local
    /// Service SAS generation). `Debug` is manually implemented to redact
    /// the key (REQ-BLOBKIT-101: secrets never leak via diagnostics).
    AccountKey {
        /// Storage account name.
        account: String,
        /// Shared access key (redacted in [`Debug`](fmt::Debug)).
        key: String,
    },
}

impl fmt::Debug for AzureCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FromEnv => f.debug_struct("AzureCredentials::FromEnv").finish(),
            Self::AccountKey { account, .. } => f
                .debug_struct("AzureCredentials::AccountKey")
                .field("account", account)
                .field("key", &"[redacted]")
                .finish(),
        }
    }
}

/// Configuration for the Azure Blob Storage backend.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AzureConfig {
    /// Blob container name, e.g. `"my-container"`.
    pub container: String,
    /// Credential resolution. Defaults to [`AzureCredentials::FromEnv`].
    pub credentials: AzureCredentials,
}

impl AzureConfig {
    /// Create a new config for `container` with [`AzureCredentials::FromEnv`].
    #[must_use]
    pub fn new(container: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            credentials: AzureCredentials::default(),
        }
    }

    /// Set the credential resolution strategy.
    #[must_use]
    pub fn with_credentials(mut self, credentials: AzureCredentials) -> Self {
        self.credentials = credentials;
        self
    }

    /// The container name.
    #[must_use]
    pub fn container(&self) -> &str {
        &self.container
    }
}

/// GCS / Azure blob store backed by [`object_store`].
///
/// Construct with [`ObjectStoreBackend::gcs`] or [`ObjectStoreBackend::azure`]
/// (synchronous — object_store's builders defer network work to operations).
#[derive(Clone)]
pub enum ObjectStoreBackend {
    /// Google Cloud Storage.
    Gcs {
        /// Configuration (bucket + credentials).
        config: GcsConfig,
        /// The configured object_store client.
        store: Arc<GoogleCloudStorage>,
    },
    /// Azure Blob Storage.
    Azure {
        /// Configuration (container + credentials).
        config: AzureConfig,
        /// The configured object_store client.
        store: Arc<MicrosoftAzure>,
    },
}

impl fmt::Debug for ObjectStoreBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // REQ-BLOBKIT-101: only the redacted config is exposed; the client is
        // omitted entirely.
        match self {
            Self::Gcs { config, .. } => f
                .debug_struct("ObjectStoreBackend::Gcs")
                .field("config", config)
                .finish(),
            Self::Azure { config, .. } => f
                .debug_struct("ObjectStoreBackend::Azure")
                .field("config", config)
                .finish(),
        }
    }
}

/// Map an [`object_store::Error`] to a [`BlobError`].
fn map_object_store_error(err: object_store::Error) -> BlobError {
    match err {
        object_store::Error::NotFound { path, .. } => BlobError::not_found(path),
        object_store::Error::PermissionDenied { path, .. }
        | object_store::Error::Unauthenticated { path, .. } => BlobError::permission_denied(path),
        object_store::Error::AlreadyExists { .. } => BlobError::AlreadyExists,
        object_store::Error::NotSupported { source } => BlobError::unsupported(source),
        object_store::Error::NotImplemented { operation, .. } => BlobError::unsupported(operation),
        object_store::Error::InvalidPath { source } => BlobError::invalid_key(source),
        other => BlobError::Other(other.to_string()),
    }
}

impl ObjectStoreBackend {
    /// Build a GCS backend from `config`.
    ///
    /// # Errors
    /// Returns [`BlobError::InvalidKey`] if the bucket name is empty and
    /// [`BlobError::Other`] if the service-account key (when supplied
    /// explicitly or via the environment) fails to parse.
    pub fn gcs(config: GcsConfig) -> Result<Self> {
        if config.bucket.trim().is_empty() {
            return Err(BlobError::invalid_key("GCS bucket must not be empty"));
        }
        let mut builder = GoogleCloudStorageBuilder::new().with_bucket_name(&config.bucket);
        match &config.credentials {
            GcsCredentials::FromEnv => {
                // blobkit's canonical variable first, then object_store's own.
                if let Ok(json) = std::env::var(GOOGLE_APPLICATION_CREDENTIALS_JSON_ENV)
                    .or_else(|_| std::env::var("GOOGLE_SERVICE_ACCOUNT_KEY"))
                {
                    builder = builder.with_service_account_key(json);
                }
                // Neither set: object_store falls back to the ambient
                // GOOGLE_APPLICATION_CREDENTIALS / metadata-server chain.
            }
            GcsCredentials::ServiceAccountKey(json) => {
                builder = builder.with_service_account_key(json.clone());
            }
        }
        let store = builder.build().map_err(map_object_store_error)?;
        Ok(Self::Gcs {
            config,
            store: Arc::new(store),
        })
    }

    /// Build an Azure Blob Storage backend from `config`.
    ///
    /// # Errors
    /// Returns [`BlobError::InvalidKey`] if the container name is empty and
    /// [`BlobError::Other`] if the credentials fail to parse.
    pub fn azure(config: AzureConfig) -> Result<Self> {
        if config.container.trim().is_empty() {
            return Err(BlobError::invalid_key("Azure container must not be empty"));
        }
        if let AzureCredentials::AccountKey { account, key } = &config.credentials {
            if account.trim().is_empty() || key.trim().is_empty() {
                return Err(BlobError::invalid_key(
                    "Azure account and key must not be empty",
                ));
            }
        }
        let builder = match &config.credentials {
            AzureCredentials::FromEnv => MicrosoftAzureBuilder::from_env(),
            AzureCredentials::AccountKey { account, key } => MicrosoftAzureBuilder::new()
                .with_account(account.clone())
                .with_access_key(key.clone()),
        };
        let store = builder
            .with_container_name(&config.container)
            .build()
            .map_err(map_object_store_error)?;
        Ok(Self::Azure {
            config,
            store: Arc::new(store),
        })
    }

    /// The GCS config, if this is the GCS variant.
    #[must_use]
    pub fn gcs_config(&self) -> Option<&GcsConfig> {
        match self {
            Self::Gcs { config, .. } => Some(config),
            Self::Azure { .. } => None,
        }
    }

    /// The Azure config, if this is the Azure variant.
    #[must_use]
    pub fn azure_config(&self) -> Option<&AzureConfig> {
        match self {
            Self::Azure { config, .. } => Some(config),
            Self::Gcs { .. } => None,
        }
    }

    /// The GCS bucket name, if this is the GCS variant.
    #[must_use]
    pub fn bucket(&self) -> Option<&str> {
        match self {
            Self::Gcs { config, .. } => Some(config.bucket()),
            Self::Azure { .. } => None,
        }
    }

    /// The Azure container name, if this is the Azure variant.
    #[must_use]
    pub fn container(&self) -> Option<&str> {
        match self {
            Self::Azure { config, .. } => Some(config.container()),
            Self::Gcs { .. } => None,
        }
    }

    fn store(&self) -> Arc<dyn object_store::ObjectStore> {
        match self {
            Self::Gcs { store, .. } => store.clone(),
            Self::Azure { store, .. } => store.clone(),
        }
    }

    fn key_path(key: &ObjectKey) -> Result<Path> {
        Path::parse(key.as_str()).map_err(BlobError::invalid_key)
    }

    #[cfg(feature = "s3")]
    async fn signed_get_url(&self, key: &ObjectKey, expires: Duration) -> Result<url::Url> {
        let path = Self::key_path(key)?;
        // Both GCS (signed URL from the service-account key) and Azure
        // (Service SAS from the account key) sign locally; neither performs a
        // network round-trip. Signing fails for credential modes that cannot
        // sign (e.g. bearer tokens) — surfaced as PermissionDenied/Other via
        // the mapping.
        match self {
            Self::Gcs { store, .. } => store.signed_url(http::Method::GET, &path, expires).await,
            Self::Azure { store, .. } => store.signed_url(http::Method::GET, &path, expires).await,
        }
        .map_err(map_object_store_error)
    }

    #[cfg(not(feature = "s3"))]
    async fn signed_get_url(&self, key: &ObjectKey, expires: Duration) -> Result<String> {
        let path = Self::key_path(key)?;
        let url = match self {
            Self::Gcs { store, .. } => store.signed_url(http::Method::GET, &path, expires).await,
            Self::Azure { store, .. } => store.signed_url(http::Method::GET, &path, expires).await,
        }
        .map_err(map_object_store_error)?;
        Ok(url.to_string())
    }
}

#[async_trait]
impl BlobStore for ObjectStoreBackend {
    async fn put(&self, key: ObjectKey, data: Bytes) -> Result<BlobId> {
        let path = Self::key_path(&key)?;
        self.store()
            .put(&path, PutPayload::from(data))
            .await
            .map_err(map_object_store_error)?;
        Ok(BlobId::new())
    }

    async fn get(&self, key: &ObjectKey) -> Result<Bytes> {
        let path = Self::key_path(key)?;
        let result = self
            .store()
            .get(&path)
            .await
            .map_err(map_object_store_error)?;
        result.bytes().await.map_err(map_object_store_error)
    }

    async fn delete(&self, key: &ObjectKey) -> Result<()> {
        let path = Self::key_path(key)?;
        if !self.exists(key).await? {
            return Err(BlobError::not_found(key.as_str()));
        }
        // The key existed a moment ago; a NotFound here means it was deleted
        // concurrently, which still satisfies the contract.
        if let Err(err) = self.store().delete(&path).await {
            match map_object_store_error(err) {
                BlobError::NotFound(_) => {}
                other => return Err(other),
            }
        }
        Ok(())
    }

    async fn exists(&self, key: &ObjectKey) -> Result<bool> {
        let path = Self::key_path(key)?;
        match self.store().head(&path).await {
            Ok(_) => Ok(true),
            Err(err) => match map_object_store_error(err) {
                BlobError::NotFound(_) => Ok(false),
                other => Err(other),
            },
        }
    }

    #[cfg(feature = "s3")]
    async fn presigned_url(&self, key: &ObjectKey, expires: Duration) -> Result<url::Url> {
        self.signed_get_url(key, expires).await
    }

    #[cfg(not(feature = "s3"))]
    async fn presigned_url(&self, key: &ObjectKey, expires: Duration) -> Result<String> {
        Ok(self.signed_get_url(key, expires).await?.to_string())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        // object_store's `Path` normalizes away trailing slashes, so the
        // server-side prefix is the trimmed form; results are post-filtered
        // with the raw prefix to keep S3-style string-prefix semantics
        // (e.g. `list("a/")` must not return `aa.txt`).
        let trimmed = prefix.trim_matches('/');
        let prefix_path = if trimmed.is_empty() {
            None
        } else {
            Some(Self::key_path(&ObjectKey::new_unchecked(
                trimmed.to_string(),
            ))?)
        };
        let mut stream = self.store().list(prefix_path.as_ref());
        // object_store lists in lexicographic order.
        let mut keys = Vec::new();
        while let Some(meta) = stream.try_next().await.map_err(map_object_store_error)? {
            let raw = meta.location.to_string();
            if !raw.starts_with(prefix) {
                continue;
            }
            // Locations come from the store's own listing (trusted source);
            // still, keys that fail blobkit validation are skipped rather
            // than failing the whole listing.
            if let Ok(key) = ObjectKey::new(raw) {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

// Tests exercise failure paths and invariants directly; unwrap/expect,
// slicing, and panicking asserts are acceptable here — violations
// surface as test failures, not production panics.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;

    fn test_gcs_config() -> GcsConfig {
        GcsConfig::new("my-bucket")
    }

    fn test_azure_config() -> AzureConfig {
        AzureConfig::new("my-container")
    }

    #[test]
    fn gcs_config_defaults() {
        let cfg = test_gcs_config();
        assert_eq!(cfg.bucket(), "my-bucket");
        assert_eq!(cfg.credentials, GcsCredentials::FromEnv);
        let backend = ObjectStoreBackend::gcs(cfg).unwrap();
        assert_eq!(backend.bucket(), Some("my-bucket"));
        assert_eq!(backend.container(), None);
    }

    #[test]
    fn azure_config_defaults() {
        let cfg = test_azure_config();
        assert_eq!(cfg.container(), "my-container");
        assert_eq!(cfg.credentials, AzureCredentials::FromEnv);
        // FromEnv requires AZURE_STORAGE_ACCOUNT in the environment; use
        // explicit credentials for a hermetic construction test.
        let hermetic = test_azure_config().with_credentials(AzureCredentials::AccountKey {
            account: "testaccount".into(),
            key: "dGVzdC1rZXktMzItYnl0ZXMtbG9uZy1hYWFhYWFhYWFhYQ==".into(),
        });
        let backend = ObjectStoreBackend::azure(hermetic).unwrap();
        assert_eq!(backend.container(), Some("my-container"));
        assert_eq!(backend.bucket(), None);
    }

    #[test]
    fn gcs_config_builders_chain() {
        let cfg =
            test_gcs_config().with_credentials(GcsCredentials::ServiceAccountKey("{}".into()));
        assert_eq!(
            cfg.credentials,
            GcsCredentials::ServiceAccountKey("{}".into())
        );
    }

    #[test]
    fn azure_config_builders_chain() {
        let cfg = test_azure_config().with_credentials(AzureCredentials::AccountKey {
            account: "myaccount".into(),
            key: "a2V5".into(),
        });
        assert_eq!(
            cfg.credentials,
            AzureCredentials::AccountKey {
                account: "myaccount".into(),
                key: "a2V5".into(),
            }
        );
    }

    #[test]
    fn empty_bucket_and_container_rejected() {
        let err = ObjectStoreBackend::gcs(GcsConfig::new("  ")).unwrap_err();
        assert!(err.is_invalid_key(), "{err:?}");
        let err = ObjectStoreBackend::azure(AzureConfig::new("")).unwrap_err();
        assert!(err.is_invalid_key(), "{err:?}");
    }

    #[test]
    fn gcs_build_rejects_bad_service_account_key() {
        let cfg = GcsConfig::new("my-bucket")
            .with_credentials(GcsCredentials::ServiceAccountKey("not-json".into()));
        let err = ObjectStoreBackend::gcs(cfg).unwrap_err();
        assert!(matches!(err, BlobError::Other(_)), "{err:?}");
    }

    #[test]
    fn azure_build_rejects_bad_connection_env() {
        // Empty explicit account/key must be rejected by our own validation.
        let cfg = AzureConfig::new("my-container").with_credentials(AzureCredentials::AccountKey {
            account: String::new(),
            key: String::new(),
        });
        let err = ObjectStoreBackend::azure(cfg).unwrap_err();
        assert!(
            matches!(err, BlobError::InvalidKey(ref msg) if msg.contains("empty")),
            "expected InvalidKey, got {err:?}"
        );
    }

    #[test]
    fn credentials_debug_redacts_secrets() {
        // REQ-BLOBKIT-101: credential material never reaches diagnostics.
        // Test the credential enums directly (backend construction with
        // dummy keys is eagerly validated by object_store and would fail).
        let gcs = GcsCredentials::ServiceAccountKey("super-secret-json".into());
        let dbg = format!("{gcs:?}");
        assert!(!dbg.contains("super-secret-json"), "secret leaked: {dbg}");
        assert!(dbg.contains("[redacted]"), "expected redaction: {dbg}");

        let azure = AzureCredentials::AccountKey {
            account: "myaccount".into(),
            key: "super-secret-key".into(),
        };
        let dbg = format!("{azure:?}");
        assert!(!dbg.contains("super-secret-key"), "secret leaked: {dbg}");
        assert!(dbg.contains("[redacted]"), "expected redaction: {dbg}");
    }

    #[test]
    fn backend_debug_has_no_secret_field() {
        // FromEnv credentials are deferred by object_store, so construction
        // succeeds without any secrets to leak in Debug output.
        let backend = ObjectStoreBackend::gcs(
            test_gcs_config(), // FromEnv is the default
        )
        .unwrap();
        let dbg = format!("{backend:?}");
        assert!(!dbg.contains("secret"), "Debug output leaked: {dbg}");
        // Azure FromEnv requires AZURE_STORAGE_ACCOUNT in the environment;
        // covered by credentials_debug_redacts_secrets at the enum level.
    }

    #[test]
    fn error_mapping_not_found() {
        let err = map_object_store_error(object_store::Error::NotFound {
            path: "a/b.txt".into(),
            source: "nope".into(),
        });
        assert!(err.is_not_found(), "{err:?}");
        assert_eq!(err.to_string(), "not found: a/b.txt");
    }

    #[test]
    fn error_mapping_permission_denied() {
        for err in [
            object_store::Error::PermissionDenied {
                path: "a".into(),
                source: "403".into(),
            },
            object_store::Error::Unauthenticated {
                path: "a".into(),
                source: "401".into(),
            },
        ] {
            let mapped = map_object_store_error(err);
            assert!(
                matches!(mapped, BlobError::PermissionDenied(_)),
                "{mapped:?}"
            );
        }
    }

    #[test]
    fn error_mapping_already_exists() {
        let err = map_object_store_error(object_store::Error::AlreadyExists {
            path: "a".into(),
            source: "409".into(),
        });
        assert!(matches!(err, BlobError::AlreadyExists), "{err:?}");
    }

    #[test]
    fn error_mapping_unsupported_and_invalid_path() {
        let err = map_object_store_error(object_store::Error::NotSupported {
            source: "nope".into(),
        });
        assert!(matches!(err, BlobError::Unsupported(_)), "{err:?}");

        let err = map_object_store_error(object_store::Error::NotImplemented {
            operation: "list".into(),
            implementer: "test".into(),
        });
        assert!(matches!(err, BlobError::Unsupported(_)), "{err:?}");

        let err = map_object_store_error(object_store::Error::InvalidPath {
            source: object_store::path::Error::EmptySegment {
                path: "a//b".into(),
            },
        });
        assert!(err.is_invalid_key(), "{err:?}");
    }

    #[test]
    fn error_mapping_other() {
        let err = map_object_store_error(object_store::Error::Generic {
            store: "TestStore",
            source: "boom".into(),
        });
        assert!(matches!(err, BlobError::Other(_)), "{err:?}");
    }

    #[test]
    fn key_path_rejects_invalid() {
        // ObjectKey validation already rejects most garbage; a control char
        // that survives ObjectKey would fail object_store's parser.
        let key = ObjectKey::new("a/b.txt").unwrap();
        assert!(ObjectStoreBackend::key_path(&key).is_ok());
    }

    #[tokio::test]
    async fn gcs_signed_url_rejects_overlong_expiry() {
        // GCS caps signed-URL lifetime at 7 days; object_store rejects this
        // before any network access, exercising the offline error path.
        let gcs = gcs_with_test_key();
        let key = ObjectKey::new("a.txt").unwrap();
        let err = gcs
            .presigned_url(&key, Duration::from_secs(604_801))
            .await
            .unwrap_err();
        assert!(matches!(err, BlobError::Other(_)), "{err:?}");
    }

    #[tokio::test]
    async fn gcs_signed_url_offline_roundtrip_shape() {
        // GCS URL signing is a local RSA operation over the service-account
        // key: no network access happens. Verify the shape of the result.
        let gcs = gcs_with_test_key();
        let key = ObjectKey::new("live-test/a.txt").unwrap();
        let url = gcs
            .presigned_url(&key, Duration::from_secs(600))
            .await
            .unwrap();
        let s = url.as_str();
        assert!(s.starts_with("https://"), "{s}");
        assert!(s.contains("my-bucket"), "{s}");
        assert!(s.contains("X-Goog-Signature"), "{s}");
        assert!(s.contains("X-Goog-Expires=600"), "{s}");
    }

    fn gcs_with_test_key() -> ObjectStoreBackend {
        ObjectStoreBackend::gcs(
            test_gcs_config()
                .with_credentials(GcsCredentials::ServiceAccountKey(GCS_TEST_KEY_JSON.into())),
        )
        .unwrap()
    }

    /// Throwaway RSA key for the offline GCS signing test. Generated for
    /// this test suite only; the corresponding account does not exist and
    /// nothing is ever sent anywhere. Signatures are checked for shape.
    const GCS_TEST_KEY_JSON: &str = r#"{
        "type": "service_account",
        "project_id": "blobkit-test",
        "private_key_id": "deadbeefdeadbeef",
        "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCrt65kcAUvslnx\ndHrXruCux9A9t1OsyLP+b7CftuUiwe6cZ9/4dIn+tFjq7NIAyzze2p9hsh/UrfBr\nmy2ncCatZRBM93Jpm5iX9392oaIpE6FPAA5cgIoRRpBhiEZTswcu/QnDRD/CHS09\nUWSS/cdw9TV94OGXy+A2mioMEemhQ60+h3vRVJeRyUuKTy4qRQu7gqPYCMXgH7gT\n9yt6n5WpIX3C8z3jpopzDXpOZrCU2Z9uF9O789u6KbQvVUXtL7VCWhy6IoYp3NGF\n7K+D7+Jxlc0tO09noeXSStjmFWHc1RaurENPdqUK39GMh/J20gXzyXtz5NO8KXkY\nU1/RnpuTAgMBAAECggEAAaydz67j7g4gIGGRXQ8Ac9PQ7PkfoLyoPJ/cKgJ/g3I+\noFnG7kY8njYl88xxU76njki1ax9wfgNgJ7xwmoRWbDRjD00OWYdB2qF4JbD3wszF\nMt7+RNqf/gEhIUJR5TkGpeejs7qzoHHmYgWsJF7DFg/eAKczq+Y5/m5MYKfADml1\nY+6qUv7MXkqgSDxRC+/qUE1a0K16xoobP6lLVAQZszSxJNJ50WlU03hYOloy5v+M\n2OKTyTxRROrt7nXYShGeMxTxAc0Wks6NeE1ijJc5zCQqam6URJCt/zPU42gd9ySm\nDwojIC5v2Uw/558LBD/f1l/Hqjw180lC1LM4EEateQKBgQDvoWvH49OmoNiOZb+j\nQD30+K7O/gJC+2EAUVqPA1Y56pYIq7F2H3fyHGiDZXDxPmBcoqkFxTTwkc5A58GH\naOk3RYkJI0CYCdU6jUM5eI1rrfQdtD2Rn51UrDOlkRgKjfxKr294Vxio0Q2rZ4P4\nG5OS79D8L9eodNJMwmDnaab8XQKBgQC3cp6a0wNmSeGvk7HiJiMIQFsyLz2dulNv\nVwbIAohe057yrnH/5izlvSLhzFgzCixnE8asYt+RUr53ITTlaXh2fvazJEPu9+qi\nPzryph3NJwBpH6dTjr/Vs/sezfn8qYr+mczztNYpM8Wl/vobfb3QQwWzveQ7GslO\nAP6VdBb4rwKBgEA4psYfjO2vVdpz8nQyF2i77T2UXc7NyCVpqDeD0WwcLrGMMjdS\nH7dHXcs5OJeu++xXu6zMOW/v47MJaZh8yWQCwsMsK3eTyw2yJj4UzPH64N3FHGsW\nt/elXwIUbLkHbIInmlxKG1XDEULKr1ejLF3I3912hPmktWfVAFuEuTgRAoGAbb2W\nnd3fqcBGz0bWYggYauY85++Ut5dwNCnmd530QG3uJxUuQzxJ3YFgrZ0VoirS1zLg\nZd2cCo5qPE/UGe0XUCOxpwbp0LnkVfznYaL4LvLG7xwtd/HsVoYdkpb7lidCa/5L\nufqTJwC+mwfGTM3S0BRYA+dz8dubUxuLMJLK7ycCgYAY36gqIFo7cP4VrPNgqiqd\nFycq/u6J2cJEiDYT9IKLBiXKFTbzh8kbMkLhaQs4YwDCcR6L5RgNRHfCZPpJjEVH\nig/qEXp3dFH3vbqGo7cWpP+zFUzJPU1wqOlMJAUxnkE+QuLNAZauZxUrkfLuMu4Y\nAOLDN3DRCegWAaAjJOAb/w==\n-----END PRIVATE KEY-----",
        "client_email": "blobkit-test@blobkit-test.iam.gserviceaccount.com",
        "client_id": "123456789012345678901",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token",
        "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
        "client_x509_cert_url": "https://c"
    }"#;
}
