// Tests exercise failure paths directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! S3 integration tests.
//!
//! Builders and offline behavior run normally with `--features s3`:
//!
//! ```text
//! cargo test --features s3 --test s3
//! ```
//!
//! A real round-trip against a live S3-compatible endpoint is available
//! behind `#[ignore]`. Configure it with environment variables and run:
//!
//! ```text
//! BLOBKIT_S3_TEST_BUCKET=my-bucket \
//! BLOBKIT_S3_REGION=us-east-1 \
//! BLOBKIT_S3_ENDPOINT=http://localhost:9000 \
//! BLOBKIT_S3_FORCE_PATH_STYLE=true \
//! BLOBKIT_S3_ACCESS_KEY=minioadmin \
//! BLOBKIT_S3_SECRET_KEY=minioadmin \
//! cargo test --features s3 --test s3 -- --ignored
//! ```
//!
//! | Variable | Required | Meaning |
//! |----------|----------|---------|
//! | `BLOBKIT_S3_TEST_BUCKET` | yes | Bucket to use for the round-trip |
//! | `BLOBKIT_S3_REGION` | no | Region (default `us-east-1`) |
//! | `BLOBKIT_S3_ENDPOINT` | no | Custom endpoint (MinIO/LocalStack/R2) |
//! | `BLOBKIT_S3_FORCE_PATH_STYLE` | no | `true` for path-style addressing |
//! | `BLOBKIT_S3_ACCESS_KEY` | no | Static access key (else AWS chain) |
//! | `BLOBKIT_S3_SECRET_KEY` | no | Static secret key (else AWS chain) |

#![cfg(feature = "s3")]

use blobkit::s3::{CredentialsMode, S3Config, S3Store};
use blobkit::store::BlobStore;
use blobkit::types::{BucketName, ObjectKey};
use bytes::Bytes;
use std::time::Duration;
use url::Url;

#[test]
fn config_defaults() {
    let cfg = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1");
    assert_eq!(cfg.bucket().as_str(), "my-bucket");
    assert_eq!(cfg.region(), "us-east-1");
    assert!(cfg.endpoint.is_none());
    assert!(!cfg.path_style);
    assert_eq!(cfg.credentials_mode, CredentialsMode::FromEnv);
    assert_eq!(cfg.timeout, Some(Duration::from_secs(30)));
}

#[test]
fn config_builders_chain() {
    let cfg = S3Config::new(BucketName::new("minio-bucket").unwrap(), "us-east-1")
        .with_endpoint(Url::parse("http://localhost:9000").unwrap())
        .with_path_style(true)
        .with_credentials(CredentialsMode::Static {
            access_key: "minioadmin".into(),
            secret_key: "minioadmin".into(),
        })
        .with_timeout(Some(Duration::from_secs(10)));

    assert_eq!(
        cfg.endpoint.as_ref().map(Url::as_str),
        Some("http://localhost:9000/")
    );
    assert!(cfg.path_style);
    assert_eq!(
        cfg.credentials_mode,
        CredentialsMode::Static {
            access_key: "minioadmin".into(),
            secret_key: "minioadmin".into(),
        }
    );
    assert_eq!(cfg.timeout, Some(Duration::from_secs(10)));
}

#[tokio::test]
async fn store_construction_offline() {
    // Static credentials avoid any environment/network lookups.
    let cfg = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1")
        .with_credentials(CredentialsMode::Static {
            access_key: "test".into(),
            secret_key: "test".into(),
        })
        .with_timeout(Some(Duration::from_secs(2)));
    let store = S3Store::new(cfg).await.unwrap();
    assert_eq!(store.config().bucket().as_str(), "my-bucket");
}

/// Live round-trip: put → exists → get → presigned_url → delete → exists.
/// Ignored by default; see the module docs for required env vars.
#[tokio::test]
#[ignore = "requires a live S3-compatible endpoint; see tests/s3.rs docs"]
async fn live_roundtrip() {
    let bucket = std::env::var("BLOBKIT_S3_TEST_BUCKET")
        .expect("BLOBKIT_S3_TEST_BUCKET must be set for the live S3 test");
    let region = std::env::var("BLOBKIT_S3_REGION").unwrap_or_else(|_| "us-east-1".into());

    let mut cfg = S3Config::new(BucketName::new(bucket).unwrap(), region);
    if let Ok(endpoint) = std::env::var("BLOBKIT_S3_ENDPOINT") {
        cfg = cfg.with_endpoint(Url::parse(&endpoint).unwrap());
    }
    if std::env::var("BLOBKIT_S3_FORCE_PATH_STYLE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
    {
        cfg = cfg.with_path_style(true);
    }
    if let (Ok(ak), Ok(sk)) = (
        std::env::var("BLOBKIT_S3_ACCESS_KEY"),
        std::env::var("BLOBKIT_S3_SECRET_KEY"),
    ) {
        cfg = cfg.with_credentials(CredentialsMode::Static {
            access_key: ak,
            secret_key: sk,
        });
    }

    let store = S3Store::new(cfg).await.unwrap();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let key = ObjectKey::new(format!("blobkit-live-test/{unique}.txt")).unwrap();
    let data = Bytes::from("hello from blobkit");

    store.put(key.clone(), data.clone()).await.unwrap();
    assert!(store.exists(&key).await.unwrap());
    assert_eq!(store.get(&key).await.unwrap(), data);

    let url = store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap();
    assert!(url.as_str().starts_with("http"));
    assert!(!url.query().unwrap_or_default().is_empty());

    store.delete(&key).await.unwrap();
    assert!(!store.exists(&key).await.unwrap());
    assert!(store.get(&key).await.unwrap_err().is_not_found());
}
