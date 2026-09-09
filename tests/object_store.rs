// Tests exercise failure paths directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! GCS / Azure (object_store facade) integration tests.
//!
//! Builders, config construction, and offline error paths run normally with
//! `--features object-store`:
//!
//! ```text
//! cargo test --features object-store --test object_store
//! ```
//!
//! Live round-trips are available behind `#[ignore]`. GCS:
//!
//! ```text
//! BLOBKIT_GCS_BUCKET=my-bucket \
//! BLOBKIT_GCS_SERVICE_ACCOUNT_KEY_JSON="$(cat service-account.json)" \
//! cargo test --features object-store --test object_store -- --ignored
//! ```
//!
//! | Variable | Required | Meaning |
//! |----------|----------|---------|
//! | `BLOBKIT_GCS_BUCKET` | yes | GCS bucket for the round-trip |
//! | `BLOBKIT_GCS_SERVICE_ACCOUNT_KEY_JSON` | no | Service-account key JSON (else ambient ADC / metadata server) |
//!
//! Azure:
//!
//! ```text
//! BLOBKIT_AZURE_CONTAINER=my-container \
//! AZURE_STORAGE_ACCOUNT=myaccount \
//! AZURE_STORAGE_ACCOUNT_KEY="$(cat azure.key)" \
//! cargo test --features object-store --test object_store -- --ignored
//! ```
//!
//! | Variable | Required | Meaning |
//! |----------|----------|---------|
//! | `BLOBKIT_AZURE_CONTAINER` | yes | Container for the round-trip |
//! | `AZURE_STORAGE_ACCOUNT` + `AZURE_STORAGE_ACCOUNT_KEY` | no | Shared-key creds (else ambient `AZURE_*` / MSI) |

#![cfg(all(feature = "object-store", not(target_arch = "wasm32")))]

use blobkit::object_store_backend::{
    AzureConfig, AzureCredentials, GcsConfig, GcsCredentials, ObjectStoreBackend,
};
use blobkit::store::BlobStore;
use blobkit::types::ObjectKey;
use bytes::Bytes;
use std::time::Duration;

#[test]
fn config_defaults() {
    let gcs = GcsConfig::new("my-bucket");
    assert_eq!(gcs.bucket(), "my-bucket");
    assert_eq!(gcs.credentials, GcsCredentials::FromEnv);

    let azure = AzureConfig::new("my-container");
    assert_eq!(azure.container(), "my-container");
    assert_eq!(azure.credentials, AzureCredentials::FromEnv);
}

#[test]
fn config_builders_chain() {
    let gcs = GcsConfig::new("my-bucket")
        .with_credentials(GcsCredentials::ServiceAccountKey("{\"k\":1}".into()));
    assert_eq!(
        gcs.credentials,
        GcsCredentials::ServiceAccountKey("{\"k\":1}".into())
    );

    let azure = AzureConfig::new("my-container").with_credentials(AzureCredentials::AccountKey {
        account: "myaccount".into(),
        key: "a2V5".into(),
    });
    assert_eq!(
        azure.credentials,
        AzureCredentials::AccountKey {
            account: "myaccount".into(),
            key: "a2V5".into(),
        }
    );
}

#[test]
fn store_construction_offline() {
    // Azure with explicit (bogus but well-formed) credentials never touches
    // the network: the builder only records configuration.
    let azure = ObjectStoreBackend::azure(AzureConfig::new("my-container").with_credentials(
        AzureCredentials::AccountKey {
            account: "devstoreaccount1".into(),
            key: "a2V5".into(),
        },
    ))
    .unwrap();
    assert_eq!(azure.container(), Some("my-container"));
    assert!(azure.gcs_config().is_none());
}

#[test]
fn debug_redacts_credentials() {
    // REQ-BLOBKIT-101: secret material never reaches diagnostics.
    // Test the credential enums directly — backend construction with
    // invalid credentials is eagerly rejected by object_store.
    let azure = AzureCredentials::AccountKey {
        account: "myaccount".into(),
        key: "super-secret-key".into(),
    };
    let dbg = format!("{azure:?}");
    assert!(!dbg.contains("super-secret-key"), "secret leaked: {dbg}");
    assert!(dbg.contains("[redacted]"), "expected redaction: {dbg}");

    let gcs = GcsCredentials::ServiceAccountKey("super-secret".into());
    let dbg = format!("{gcs:?}");
    assert!(!dbg.contains("super-secret"), "secret leaked: {dbg}");
    assert!(dbg.contains("[redacted]"), "expected redaction: {dbg}");
}

/// Live round-trip against GCS: put → exists → get → presigned_url →
/// list → delete → exists. Ignored by default; see module docs for env vars.
#[tokio::test]
#[ignore = "requires a live GCS bucket; see tests/object_store.rs docs"]
async fn live_gcs_roundtrip() {
    let bucket = std::env::var("BLOBKIT_GCS_BUCKET")
        .expect("BLOBKIT_GCS_BUCKET must be set for the live GCS test");
    let mut cfg = GcsConfig::new(bucket);
    if let Ok(json) = std::env::var("BLOBKIT_GCS_SERVICE_ACCOUNT_KEY_JSON") {
        cfg = cfg.with_credentials(GcsCredentials::ServiceAccountKey(json));
    }
    let store = ObjectStoreBackend::gcs(cfg).unwrap();

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!("blobkit-live-test/{unique}");
    let key = ObjectKey::new(format!("{prefix}/hello.txt")).unwrap();
    let data = Bytes::from("hello from blobkit (gcs)");

    store.put(key.clone(), data.clone()).await.unwrap();
    assert!(store.exists(&key).await.unwrap());
    assert_eq!(store.get(&key).await.unwrap(), data);

    let url = store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap();
    #[cfg(feature = "s3")]
    assert!(url.as_str().contains("X-Goog-Signature"));
    #[cfg(not(feature = "s3"))]
    assert!(url.contains("X-Goog-Signature"));

    let listed = store.list(&prefix).await.unwrap();
    assert!(
        listed.contains(&key),
        "listed {listed:?} should contain {key}"
    );

    store.delete(&key).await.unwrap();
    assert!(!store.exists(&key).await.unwrap());
    assert!(store.get(&key).await.unwrap_err().is_not_found());
    assert!(store.delete(&key).await.unwrap_err().is_not_found());
}

/// Live round-trip against Azure Blob Storage. Ignored by default; see
/// module docs for env vars.
#[tokio::test]
#[ignore = "requires a live Azure container; see tests/object_store.rs docs"]
async fn live_azure_roundtrip() {
    let container = std::env::var("BLOBKIT_AZURE_CONTAINER")
        .expect("BLOBKIT_AZURE_CONTAINER must be set for the live Azure test");
    let mut cfg = AzureConfig::new(container);
    if let (Ok(account), Ok(key)) = (
        std::env::var("AZURE_STORAGE_ACCOUNT"),
        std::env::var("AZURE_STORAGE_ACCOUNT_KEY"),
    ) {
        cfg = cfg.with_credentials(AzureCredentials::AccountKey { account, key });
    }
    let store = ObjectStoreBackend::azure(cfg).unwrap();

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!("blobkit-live-test/{unique}");
    let key = ObjectKey::new(format!("{prefix}/hello.txt")).unwrap();
    let data = Bytes::from("hello from blobkit (azure)");

    store.put(key.clone(), data.clone()).await.unwrap();
    assert!(store.exists(&key).await.unwrap());
    assert_eq!(store.get(&key).await.unwrap(), data);

    let url = store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap();
    // Azure Service SAS carries `sig=`.
    #[cfg(feature = "s3")]
    assert!(url.as_str().contains("sig="));
    #[cfg(not(feature = "s3"))]
    assert!(url.contains("sig="));

    let listed = store.list(&prefix).await.unwrap();
    assert!(
        listed.contains(&key),
        "listed {listed:?} should contain {key}"
    );

    store.delete(&key).await.unwrap();
    assert!(!store.exists(&key).await.unwrap());
    assert!(store.get(&key).await.unwrap_err().is_not_found());
    assert!(store.delete(&key).await.unwrap_err().is_not_found());
}
