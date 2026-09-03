//! S3 backend stub.
//!
//! For `v0.1` the heavy `aws-sdk-s3` / `aws-config` dependencies are intentionally
//! omitted to keep compile times fast and to avoid pulling large AWS trees into
//! every consumer. This module therefore provides only the *configuration* and
//! type stubs required so that call sites can be migrated to `blobkit` without
//! waiting for a full S3 implementation.
//!
//! All [`BlobStore`] methods return [`BlobError::Unsupported`] until the real
//! client is implemented. The stub is fully documented so that future work
//! (wiring `aws-sdk-s3` behind the `s3` feature) is straightforward.
//!
//! # Migration
//!
//! Replace:
//!
//! ```rust,ignore
//! // before
//! let client = S3Client::from_conf(&conf).await;
//! ```
//!
//! With:
//!
//! ```rust,no_run
//! use blobkit::s3::{S3Config, S3Store};
//! use blobkit::types::BucketName;
//!
//! let cfg = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1");
//! let store = S3Store::new(cfg);
//! // store.put(...).await  // returns Unsupported until implemented
//! ```

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use async_trait::async_trait;
use bytes::Bytes;
use core::time::Duration;

use crate::error::{BlobError, Result};
use crate::store::BlobStore;
use crate::types::{BlobId, BucketName, ObjectKey};

#[cfg(feature = "s3")]
use url::Url;

/// Configuration for the S3 backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Target bucket.
    pub bucket: BucketName,
    /// AWS region, e.g. `"us-east-1"`.
    pub region: String,
    /// Optional custom endpoint (e.g. for MinIO, LocalStack, or R2).
    #[cfg(feature = "s3")]
    pub endpoint: Option<Url>,
    /// Optional custom endpoint as string when `s3` feature is disabled.
    #[cfg(not(feature = "s3"))]
    pub endpoint: Option<String>,
}

impl S3Config {
    /// Create a new config with the given bucket and region.
    #[must_use]
    pub fn new(bucket: BucketName, region: impl Into<String>) -> Self {
        Self {
            bucket,
            region: region.into(),
            endpoint: None,
        }
    }

    /// Set a custom endpoint.
    ///
    /// Use this for S3-compatible stores (MinIO, SeaweedFS, Cloudflare R2).
    #[cfg(feature = "s3")]
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: Url) -> Self {
        self.endpoint = Some(endpoint);
        self
    }

    /// Set a custom endpoint (string form, when `s3` feature is disabled).
    #[cfg(not(feature = "s3"))]
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// The bucket name.
    #[must_use]
    pub fn bucket(&self) -> &BucketName {
        &self.bucket
    }

    /// The region.
    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }
}

/// S3-backed blob store (stub).
///
/// The struct holds only configuration in `v0.1`. All operations return
/// `Err(BlobError::Unsupported)` with a message indicating that the S3
/// backend is not yet implemented. This allows call sites to compile against
/// the unified [`BlobStore`] trait today and swap in a real client later
/// without changing their own code.
///
/// A real implementation will store an `aws_sdk_s3::Client` and delegate
/// `put` → `PutObject`, `get` → `GetObject`, etc., and generate pre-signed
/// URLs via `aws_sdk_s3::presigning`.
#[derive(Debug, Clone)]
pub struct S3Store {
    config: S3Config,
}

impl S3Store {
    /// Create a new stub store from `config`.
    #[must_use]
    pub fn new(config: S3Config) -> Self {
        Self { config }
    }

    /// Borrow the config.
    #[must_use]
    pub fn config(&self) -> &S3Config {
        &self.config
    }

    /// Create a config and store in one call.
    #[must_use]
    pub fn from_bucket_region(bucket: BucketName, region: impl Into<String>) -> Self {
        Self::new(S3Config::new(bucket, region))
    }
}

#[async_trait]
impl BlobStore for S3Store {
    async fn put(&self, _key: ObjectKey, _data: Bytes) -> Result<BlobId> {
        Err(BlobError::unsupported(
            "S3Store::put is not implemented in v0.1 (stub)",
        ))
    }

    async fn get(&self, _key: &ObjectKey) -> Result<Bytes> {
        Err(BlobError::unsupported(
            "S3Store::get is not implemented in v0.1 (stub)",
        ))
    }

    async fn delete(&self, _key: &ObjectKey) -> Result<()> {
        Err(BlobError::unsupported(
            "S3Store::delete is not implemented in v0.1 (stub)",
        ))
    }

    async fn exists(&self, _key: &ObjectKey) -> Result<bool> {
        Err(BlobError::unsupported(
            "S3Store::exists is not implemented in v0.1 (stub)",
        ))
    }

    #[cfg(feature = "s3")]
    async fn presigned_url(&self, _key: &ObjectKey, _expires: Duration) -> Result<Url> {
        Err(BlobError::unsupported(
            "S3Store::presigned_url is not implemented in v0.1 (stub)",
        ))
    }

    #[cfg(not(feature = "s3"))]
    async fn presigned_url(&self, _key: &ObjectKey, _expires: Duration) -> Result<String> {
        Err(BlobError::unsupported(
            "S3Store::presigned_url is not implemented in v0.1 (stub)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> S3Config {
        S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1")
    }

    #[test]
    fn config_new() {
        let cfg = test_config();
        assert_eq!(cfg.bucket().as_str(), "my-bucket");
        assert_eq!(cfg.region(), "us-east-1");
        assert!(cfg.endpoint.is_none());
    }

    #[test]
    fn config_with_endpoint() {
        #[cfg(feature = "s3")]
        {
            let url = Url::parse("https://s3.example.com").unwrap();
            let cfg = test_config().with_endpoint(url.clone());
            assert_eq!(cfg.endpoint, Some(url));
        }
        #[cfg(not(feature = "s3"))]
        {
            let cfg = test_config().with_endpoint("https://s3.example.com");
            assert_eq!(cfg.endpoint.as_deref(), Some("https://s3.example.com"));
        }
    }

    #[tokio::test]
    async fn stub_returns_unsupported() {
        use crate::store::BlobStore;
        let store = S3Store::new(test_config());
        let key = ObjectKey::new("hello.txt").unwrap();
        let err = store.put(key.clone(), Bytes::from("hi")).await.unwrap_err();
        assert!(matches!(err, BlobError::Unsupported(_)));
        let err = store.get(&key).await.unwrap_err();
        assert!(matches!(err, BlobError::Unsupported(_)));
        let err = store.delete(&key).await.unwrap_err();
        assert!(matches!(err, BlobError::Unsupported(_)));
        let err = store.exists(&key).await.unwrap_err();
        assert!(matches!(err, BlobError::Unsupported(_)));
        let err = store
            .presigned_url(&key, Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(matches!(err, BlobError::Unsupported(_)));
    }
}
