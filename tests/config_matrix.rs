//! Config-knob behavior matrix for blobkit.
//!
//! Every public knob must OBSERVABLY change behavior: each test pairs a
//! default with an alternate value and asserts the observable output
//! differs.
//!
//! Knobs covered (10):
//!   1. `ObjectKey` validation (length boundary, traversal guard)
//!   2. `BucketName` validation (length edges)
//!   3. `guess_content_type` (mime mapping; feature-off fallback) — the
//!      feature-off path was a GAP in `tests/coverage_gates.rs`
//!   4. `compute_sha256` (known vector; feature-off `None`) — off path GAP
//!   5. `BlobMetadata::content_type` recorded per put (extension-driven)
//!   6. `sha256` digest recorded per put (matches `compute_sha256`)
//!   7. `LocalStore` root + `max_bytes` quota
//!   8. `S3Config` endpoint / path-style / bucket shape presigned URLs
//!      offline — GAP (previously only live-ignored coverage)
//!   9. `S3Config` timeout None vs Some (construction + config roundtrip)
//!  10. `S3Config` credentials mode (static offline construction;
//!      redaction proved in `tests/coverage_gates.rs`)
//!
//! Already covered elsewhere (cited, not duplicated): `BlobId`
//! uniqueness/roundtrip, boxed-store delegation, object-store backend
//! config, io-uring backend roundtrip, local error paths.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

#[cfg(feature = "std")]
use bytes::Bytes;

#[cfg(feature = "std")]
use blobkit::memory::MemoryStore;
#[cfg(feature = "std")]
use blobkit::store::BlobStore;
use blobkit::types::{BucketName, ObjectKey};

// ---------------------------------------------------------------------------
// 1. ObjectKey: length boundary + traversal guard enforced at the store
// ---------------------------------------------------------------------------

#[test]
fn knob_object_key_length_boundary() {
    assert!(ObjectKey::new("a".repeat(1024)).is_ok());
    assert!(ObjectKey::new("a".repeat(1025)).is_err());
}

#[cfg(feature = "std")]
#[tokio::test]
async fn knob_object_key_traversal_rejected_by_store() {
    let store = MemoryStore::new();
    // `..` as a path component never reaches the backend...
    assert!(ObjectKey::new("../escape").is_err());
    assert!(ObjectKey::new("a/../b").is_err());
    // ...while lookalikes without a `..` component flow through.
    let ok = ObjectKey::new("file..name.txt").unwrap();
    store
        .put(ok.clone(), Bytes::from_static(b"x"))
        .await
        .unwrap();
    assert!(store.exists(&ok).await.unwrap());
}

// ---------------------------------------------------------------------------
// 2. BucketName: length edges
// ---------------------------------------------------------------------------

#[test]
fn knob_bucket_name_length_edges() {
    assert!(BucketName::new("ab").is_err(), "2 chars: too short");
    assert!(BucketName::new("abc").is_ok());
    assert!(BucketName::new("a".repeat(63)).is_ok());
    assert!(
        BucketName::new("a".repeat(64)).is_err(),
        "64 chars: too long"
    );
}

// ---------------------------------------------------------------------------
// 3. guess_content_type: mapping on / octet-stream fallback off
// ---------------------------------------------------------------------------

#[cfg(feature = "mime_guess")]
#[test]
fn knob_guess_content_type_maps_extensions() {
    use blobkit::types::guess_content_type;
    assert_eq!(
        guess_content_type(&ObjectKey::new("a/b.png").unwrap()),
        "image/png"
    );
    assert_eq!(
        guess_content_type(&ObjectKey::new("a/b.txt").unwrap()),
        "text/plain"
    );
}

#[cfg(not(feature = "mime_guess"))]
#[test]
fn knob_guess_content_type_without_feature_falls_back() {
    use blobkit::types::guess_content_type;
    // GAP FILL: the no-`mime_guess` path always yields octet-stream.
    assert_eq!(
        guess_content_type(&ObjectKey::new("a/b.png").unwrap()),
        "application/octet-stream"
    );
}

// ---------------------------------------------------------------------------
// 4. compute_sha256: known vector on / None off
// ---------------------------------------------------------------------------

#[cfg(feature = "sha2")]
#[test]
fn knob_compute_sha256_known_vector() {
    use blobkit::types::compute_sha256;
    assert_eq!(
        compute_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(compute_sha256(b"abc").len(), 64);
}

#[cfg(not(feature = "sha2"))]
#[test]
fn knob_compute_sha256_without_feature_is_none() {
    use blobkit::types::compute_sha256;
    // GAP FILL: the no-`sha2` stub returns None.
    assert_eq!(compute_sha256(b"abc"), None);
}

// ---------------------------------------------------------------------------
// 5+6. Store-recorded metadata: content type + sha256 follow the knobs
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
#[tokio::test]
async fn knob_memory_put_records_content_type_and_sha256() {
    let store = MemoryStore::new();

    let png = ObjectKey::new("img/photo.png").unwrap();
    store
        .put(png.clone(), Bytes::from_static(b"png-bytes"))
        .await
        .unwrap();
    let meta = store.metadata(&png).expect("metadata recorded");
    assert_eq!(meta.size, 9);
    #[cfg(feature = "mime_guess")]
    assert_eq!(meta.content_type, "image/png");
    #[cfg(not(feature = "mime_guess"))]
    assert_eq!(meta.content_type, "application/octet-stream");

    #[cfg(feature = "sha2")]
    {
        use blobkit::types::compute_sha256;
        assert_eq!(
            meta.sha256.as_deref(),
            Some(compute_sha256(b"png-bytes").as_str()),
            "recorded digest must match compute_sha256"
        );
    }

    // Unknown extension → generic binary type (mapping knob, not hardcoded).
    let bin = ObjectKey::new("obj/blob.unknownext").unwrap();
    store
        .put(bin.clone(), Bytes::from_static(b"data"))
        .await
        .unwrap();
    let meta = store.metadata(&bin).unwrap();
    assert_eq!(meta.content_type, "application/octet-stream");
}

// ---------------------------------------------------------------------------
// 7. LocalStore root + max_bytes quota
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
#[tokio::test]
async fn knob_local_max_bytes_contrast() {
    use blobkit::local::LocalStore;

    let dir = tempfile::tempdir().unwrap();
    let capped = LocalStore::new_unchecked(dir.path().to_path_buf()).with_max_bytes(Some(8));
    let key = ObjectKey::new("big/data.bin").unwrap();
    let err = capped
        .put(key, Bytes::from(vec![0u8; 9]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, blobkit::error::BlobError::StorageFull),
        "9 bytes over an 8-byte quota: {err}"
    );

    let open = LocalStore::new_unchecked(dir.path().to_path_buf()).with_max_bytes(None);
    let key = ObjectKey::new("big/data.bin").unwrap();
    open.put(key.clone(), Bytes::from(vec![0u8; 9]))
        .await
        .unwrap();
    assert!(open.exists(&key).await.unwrap());
}

// ---------------------------------------------------------------------------
// 8+9+10. S3Config knobs: offline presigned-URL shape + construction
// ---------------------------------------------------------------------------

#[cfg(feature = "s3")]
mod s3_knobs {
    use super::*;
    use std::time::Duration;

    use blobkit::s3::{CredentialsMode, S3Config, S3Store};
    use url::Url;

    #[tokio::test]
    async fn knob_s3_endpoint_and_path_style_shape_presigned_url() {
        // Custom endpoint + path style: bucket appears as path segments.
        let custom = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1")
            .with_endpoint(Url::parse("http://localhost:9000").unwrap())
            .with_path_style(true)
            .with_credentials(CredentialsMode::Static {
                access_key: "test".into(),
                secret_key: "test".into(),
            });
        let store = S3Store::new(custom).await.unwrap();
        let url = store
            .presigned_url(
                &ObjectKey::new("docs/report.pdf").unwrap(),
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        let s = url.as_str();
        assert!(s.contains("localhost:9000"), "endpoint host: {s}");
        assert!(s.contains("my-bucket"), "bucket: {s}");
        assert!(
            s.contains("docs/report.pdf") || s.contains("docs%2Freport.pdf"),
            "key: {s}"
        );

        // Default addressing (no endpoint override): virtual-host style
        // against the real regional endpoint — still fully offline.
        let hosted = S3Config::new(BucketName::new("my-bucket").unwrap(), "eu-west-1")
            .with_credentials(CredentialsMode::Static {
                access_key: "test".into(),
                secret_key: "test".into(),
            });
        let store = S3Store::new(hosted).await.unwrap();
        let url = store
            .presigned_url(
                &ObjectKey::new("docs/report.pdf").unwrap(),
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        let s = url.as_str();
        assert!(
            s.contains("my-bucket") && s.contains("eu-west-1"),
            "virtual-host URL must carry bucket+region: {s}"
        );
    }

    #[test]
    fn knob_s3_timeout_none_vs_some_roundtrips() {
        let base = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1");
        assert_eq!(base.timeout, Some(Duration::from_secs(30)));
        let none = base.clone().with_timeout(None);
        assert_eq!(none.timeout, None);
        let ten = base.with_timeout(Some(Duration::from_secs(10)));
        assert_eq!(ten.timeout, Some(Duration::from_secs(10)));
    }

    #[tokio::test]
    async fn knob_s3_static_credentials_construct_offline() {
        let cfg = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1")
            .with_credentials(CredentialsMode::Static {
                access_key: "test".into(),
                secret_key: "test".into(),
            });
        let store = S3Store::new(cfg).await.unwrap();
        assert_eq!(store.config().bucket().as_str(), "my-bucket");
        // A second bucket constructs just as hermetically.
        let other = S3Config::new(BucketName::new("other-bucket").unwrap(), "us-east-1")
            .with_credentials(CredentialsMode::Static {
                access_key: "test".into(),
                secret_key: "test".into(),
            });
        let other_store = S3Store::new(other).await.unwrap();
        assert_eq!(other_store.config().bucket().as_str(), "other-bucket");
    }
}
