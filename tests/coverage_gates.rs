// Tests exercise failure paths directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Coverage-gap tests: type-conversion trait impls, builder methods, store
//! error paths, boxed-store delegation, and S3 credential redaction. Every
//! test asserts real behavior (wire values, error variants, URL shapes).

use std::str::FromStr;
use std::time::Duration;

use bytes::Bytes;

use blobkit::error::BlobError;
use blobkit::local::LocalStore;
use blobkit::memory::MemoryStore;
use blobkit::store::BlobStore;
use blobkit::types::{guess_content_type, BlobId, BlobMetadata, BucketName, ObjectKey};
use blobkit::{S3Config, S3Store};

// ---------------------------------------------------------------------------
// ObjectKey trait impls
// ---------------------------------------------------------------------------

#[test]
fn object_key_display_fromstr_tryfrom_asref_roundtrip() {
    let key = ObjectKey::new("docs/readme.md").unwrap();

    // Display renders the key verbatim.
    assert_eq!(format!("{key}"), "docs/readme.md");

    // FromStr parses valid keys.
    let parsed = ObjectKey::from_str("docs/readme.md").unwrap();
    assert_eq!(parsed.as_str(), "docs/readme.md");

    // FromStr rejects invalid keys with invalid_key errors.
    let err = ObjectKey::from_str("").unwrap_err();
    assert!(err.is_invalid_key(), "empty key must be invalid: {err}");
    let err = ObjectKey::from_str("/leading").unwrap_err();
    assert!(err.is_invalid_key());

    // TryFrom<String> and TryFrom<&str> mirror FromStr.
    let from_string = ObjectKey::try_from("a/b.txt".to_string()).unwrap();
    let from_slice = ObjectKey::try_from("a/b.txt").unwrap();
    assert_eq!(from_string, from_slice);
    assert!(ObjectKey::try_from(String::new()).is_err());

    // AsRef<str> borrows the inner key.
    let borrowed: &str = key.as_ref();
    assert_eq!(borrowed, "docs/readme.md");

    // new_unchecked trusts the caller — it must preserve the exact bytes.
    let unchecked = ObjectKey::new_unchecked("trusted/key.bin".to_string());
    assert_eq!(unchecked.as_str(), "trusted/key.bin");
    assert_eq!(unchecked.into_inner(), "trusted/key.bin");
}

// ---------------------------------------------------------------------------
// BucketName trait impls
// ---------------------------------------------------------------------------

#[test]
fn bucket_name_display_fromstr_asref_into_inner() {
    let bucket = BucketName::new("my-bucket").unwrap();
    assert_eq!(format!("{bucket}"), "my-bucket");

    let parsed = BucketName::from_str("other.bucket").unwrap_err();
    assert!(parsed.is_invalid_key(), "dots are not allowed: {parsed}");
    let ok = BucketName::from_str("ok-name").unwrap();
    assert_eq!(ok.as_str(), "ok-name");

    let borrowed: &str = bucket.as_ref();
    assert_eq!(borrowed, "my-bucket");
    assert_eq!(bucket.into_inner(), "my-bucket");

    // Length guard: <3 and >63 rejected.
    assert!(BucketName::new("ab").is_err());
    assert!(BucketName::new("a".repeat(64)).is_err());
}

// ---------------------------------------------------------------------------
// BlobId / BlobMetadata builders
// ---------------------------------------------------------------------------

#[cfg(feature = "typed-id")]
#[test]
fn blob_id_uuid_roundtrip_and_parse_errors() {
    let id = BlobId::new();
    let uuid = *id.as_uuid();
    let rebuilt = BlobId::from_uuid(uuid);
    assert_eq!(*rebuilt.as_uuid(), uuid, "from_uuid must preserve the uuid");

    // FromStr accepts canonical uuid strings and rejects garbage.
    let parsed = BlobId::from_str(&uuid.to_string()).unwrap();
    assert_eq!(*parsed.as_uuid(), uuid);
    let err = BlobId::from_str("not-a-uuid").unwrap_err();
    assert!(err.is_invalid_key(), "garbage id must be invalid: {err}");
}

#[test]
fn blob_metadata_builders_chain() {
    let key = ObjectKey::new("video/clip.mp4").unwrap();
    let meta = BlobMetadata::new(key, 1234)
        .with_content_type("video/mp4")
        .with_sha256("deadbeef");
    assert_eq!(meta.content_type, "video/mp4");
    assert_eq!(meta.sha256.as_deref(), Some("deadbeef"));
    assert_eq!(meta.size, 1234);
}

#[test]
fn guess_content_type_maps_known_extensions() {
    assert_eq!(
        guess_content_type(&ObjectKey::new("a/b.png").unwrap()),
        "image/png"
    );
    assert_eq!(
        guess_content_type(&ObjectKey::new("a/b.unknownext").unwrap()),
        "application/octet-stream"
    );
}

// ---------------------------------------------------------------------------
// BlobError conversions
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
#[test]
fn tempfile_persist_error_converts_to_io_error() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let persist_err = tempfile::PersistError {
        error: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "rename denied"),
        file,
    };
    let blob_err = BlobError::from(persist_err);
    assert!(
        matches!(blob_err, BlobError::Io(_)),
        "PersistError must map to BlobError::Io, got: {blob_err}"
    );
}

// ---------------------------------------------------------------------------
// LocalStore builders + error paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_with_limits_rejects_non_directory_root() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("plain-file");
    std::fs::write(&file_path, b"not a dir").unwrap();

    let err = LocalStore::with_limits(file_path, None).await.unwrap_err();
    assert!(
        matches!(err, BlobError::Io(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists),
        "file root must fail at create_dir_all with EEXIST, got: {err}"
    );
}

#[tokio::test]
async fn local_new_unchecked_builder_and_root_accessor() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new_unchecked(dir.path().to_path_buf()).with_max_bytes(Some(8));
    assert_eq!(
        store.root(),
        dir.path(),
        "root() must return the configured root"
    );

    // The max-bytes guard is active on the unchecked-built store.
    let key = ObjectKey::new("too/big.bin").unwrap();
    let err = store.put(key, Bytes::from(vec![0u8; 9])).await.unwrap_err();
    assert!(
        matches!(err, BlobError::StorageFull),
        "9 bytes over an 8-byte limit: {err}"
    );

    // Normal traffic still flows through the unchecked-built store.
    let key = ObjectKey::new("ok/data.bin").unwrap();
    store.put(key, Bytes::from_static(b"small")).await.unwrap();
    assert!(store
        .exists(&ObjectKey::new("ok/data.bin").unwrap())
        .await
        .unwrap());
}

#[tokio::test]
async fn local_put_fails_when_parent_component_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    // `blocker` is a file, so creating the parent dir `blocker/` fails.
    std::fs::write(dir.path().join("blocker"), b"x").unwrap();

    let key = ObjectKey::new("blocker/inner.txt").unwrap();
    let err = store
        .put(key, Bytes::from_static(b"data"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BlobError::Io(_)),
        "parent-is-file must surface as an IO error, got: {err}"
    );
}

#[tokio::test]
async fn local_put_fails_when_destination_is_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    // The destination path itself is an existing directory — the final
    // persist/rename must fail and map to BlobError.
    std::fs::create_dir_all(dir.path().join("dest")).unwrap();
    let key = ObjectKey::new("dest").unwrap();
    let err = store
        .put(key, Bytes::from_static(b"data"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BlobError::Io(_)),
        "dest-is-directory must surface as an IO error, got: {err}"
    );
}

#[tokio::test]
async fn local_put_rejects_dotdot_containing_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    // ObjectKey accepts `file..name` (no empty segments) but the store's
    // traversal guard rejects any key containing `..` — defense in depth.
    let key = ObjectKey::new("file..name.txt").unwrap();
    let err = store
        .put(key, Bytes::from_static(b"data"))
        .await
        .unwrap_err();
    assert!(
        err.is_invalid_key(),
        "keys containing '..' must be rejected by the store: {err}"
    );
}

#[tokio::test]
async fn local_presigned_url_is_file_url_with_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    let key = ObjectKey::new("signed/report.pdf").unwrap();
    store
        .put(key.clone(), Bytes::from_static(b"pdf"))
        .await
        .unwrap();

    let url = store
        .presigned_url(&key, Duration::from_secs(90))
        .await
        .unwrap();
    assert_eq!(
        url.scheme(),
        "file",
        "local presigned URLs are file URLs: {url}"
    );
    assert!(
        url.path().ends_with("signed/report.pdf"),
        "URL must point at the object path: {url}"
    );
    assert_eq!(
        url.query_pairs()
            .find(|(k, _)| k == "expires")
            .map(|(_, v)| v.to_string())
            .as_deref(),
        Some("90")
    );

    // Presigning a missing object must fail closed with not-found.
    let missing = ObjectKey::new("signed/absent.pdf").unwrap();
    let err = store
        .presigned_url(&missing, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(err.is_not_found(), "presign of missing object: {err}");
}

// ---------------------------------------------------------------------------
// Boxed store delegation (store.rs forward impls)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn boxed_dyn_store_delegates_all_operations() {
    let boxed: Box<dyn BlobStore> = Box::new(MemoryStore::new());
    let key = ObjectKey::new("boxed/data.txt").unwrap();

    boxed
        .put(key.clone(), Bytes::from_static(b"boxed"))
        .await
        .unwrap();
    let got = boxed.get(&key).await.unwrap();
    assert_eq!(got, Bytes::from_static(b"boxed"));
    assert!(boxed.exists(&key).await.unwrap());
    boxed.delete(&key).await.unwrap();
    assert!(!boxed.exists(&key).await.unwrap());

    let err = boxed.get(&key).await.unwrap_err();
    assert!(
        err.is_not_found(),
        "get after delete must be not-found: {err}"
    );
}

#[cfg(feature = "s3")]
#[tokio::test]
async fn boxed_local_store_presigns_through_trait_object() {
    let dir = tempfile::tempdir().unwrap();
    let local = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    let boxed: std::boxed::Box<dyn BlobStore> = Box::new(local);
    let key = ObjectKey::new("box/signed.txt").unwrap();
    boxed
        .put(key.clone(), Bytes::from_static(b"x"))
        .await
        .unwrap();
    let url = boxed
        .presigned_url(&key, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(url.scheme(), "file");
}

// ---------------------------------------------------------------------------
// io_uring backend (Linux + io-uring feature)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "io-uring", target_os = "linux"))]
mod io_uring {
    use super::*;
    use blobkit::io_uring_backend::store::IoUringStore;

    #[tokio::test]
    async fn ring_with_limits_rejects_non_directory_root() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("plain-file");
        std::fs::write(&file_path, b"not a dir").unwrap();

        let err = IoUringStore::with_limits(file_path, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BlobError::Io(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists),
            "file root must fail at create_dir_all with EEXIST, got: {err}"
        );
    }

    #[tokio::test]
    async fn ring_new_unchecked_and_max_bytes_guard() {
        let dir = tempfile::tempdir().unwrap();
        let store = IoUringStore::new_unchecked(dir.path().to_path_buf());
        let guarded = IoUringStore::with_limits(dir.path().to_path_buf(), Some(4))
            .await
            .unwrap();

        let key = ObjectKey::new("ring/big.bin").unwrap();
        let err = guarded
            .put(key, Bytes::from(vec![0u8; 5]))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BlobError::StorageFull),
            "5 bytes over a 4-byte limit: {err}"
        );

        // Unchecked-built store round-trips through the BlobStore trait.
        let key = ObjectKey::new("ring/data.bin").unwrap();
        store
            .put(key.clone(), Bytes::from_static(b"ring-data"))
            .await
            .unwrap();
        assert_eq!(
            store.get(&key).await.unwrap(),
            Bytes::from_static(b"ring-data")
        );
        assert!(store.exists(&key).await.unwrap());
    }

    #[tokio::test]
    async fn ring_store_rejects_dotdot_keys_and_missing_objects() {
        let dir = tempfile::tempdir().unwrap();
        let store = IoUringStore::new(dir.path().to_path_buf()).await.unwrap();

        let key = ObjectKey::new("file..name.txt").unwrap();
        let err = store.put(key, Bytes::from_static(b"x")).await.unwrap_err();
        assert!(err.is_invalid_key(), "'..' keys must be rejected: {err}");

        let missing = ObjectKey::new("ring/absent.bin").unwrap();
        let err = store.get(&missing).await.unwrap_err();
        assert!(err.is_not_found(), "get of missing object: {err}");
        let err = store.delete(&missing).await.unwrap_err();
        assert!(err.is_not_found(), "delete of missing object: {err}");
        assert!(!store.exists(&missing).await.unwrap());
    }

    #[tokio::test]
    async fn ring_put_fails_when_parent_component_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = IoUringStore::new(dir.path().to_path_buf()).await.unwrap();
        std::fs::write(dir.path().join("blocker"), b"x").unwrap();

        let key = ObjectKey::new("blocker/inner.txt").unwrap();
        let err = store
            .put(key, Bytes::from_static(b"data"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, BlobError::Io(_)),
            "parent-is-file must surface as an IO error, got: {err}"
        );
    }

    #[cfg(feature = "s3")]
    #[tokio::test]
    async fn ring_presigned_url_delegates_to_local_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let store = IoUringStore::new(dir.path().to_path_buf()).await.unwrap();
        let key = ObjectKey::new("ring/signed.bin").unwrap();
        store
            .put(key.clone(), Bytes::from_static(b"x"))
            .await
            .unwrap();
        let url = store
            .presigned_url(&key, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(url.scheme(), "file");
        assert_eq!(
            url.query_pairs()
                .find(|(k, _)| k == "expires")
                .map(|(_, v)| v.to_string()),
            Some("10".to_string())
        );

        // Delegated presigning fails closed for missing objects too.
        let missing = ObjectKey::new("ring/absent-signed.bin").unwrap();
        let err = store
            .presigned_url(&missing, Duration::from_secs(10))
            .await
            .unwrap_err();
        assert!(err.is_not_found(), "presign of missing object: {err}");
    }
}

// ---------------------------------------------------------------------------
// S3 config surface (hermetic — no network calls)
// ---------------------------------------------------------------------------

#[cfg(feature = "s3")]
mod s3_hermetic {
    use super::*;
    use blobkit::s3::CredentialsMode;

    #[test]
    fn credentials_mode_debug_redacts_static_secret() {
        let cred = CredentialsMode::Static {
            access_key: "AKIA-TEST".to_string(),
            secret_key: "super-secret-do-not-leak".to_string(),
        };
        let rendered = format!("{cred:?}");
        assert!(
            rendered.contains("AKIA-TEST"),
            "access key is not sensitive: {rendered}"
        );
        assert!(
            !rendered.contains("super-secret"),
            "secret key MUST be redacted in Debug output: {rendered}"
        );
        assert!(rendered.contains("[redacted]"));

        // The other variants render without leaking anything sensitive.
        assert!(format!("{:?}", CredentialsMode::FromEnv).contains("FromEnv"));
        assert!(format!("{:?}", CredentialsMode::FromProfile).contains("FromProfile"));
    }

    #[tokio::test]
    async fn s3_store_builds_offline_with_static_credentials_and_endpoint() {
        let config = S3Config::new(BucketName::new("test-bucket").unwrap(), "us-east-1")
            .with_endpoint(url::Url::parse("http://127.0.0.1:9").unwrap())
            .with_path_style(true)
            .with_credentials(CredentialsMode::Static {
                access_key: "test-access".to_string(),
                secret_key: "test-secret".to_string(),
            })
            .with_timeout(Some(Duration::from_secs(2)));

        // Client construction resolves credentials and region locally;
        // no request is made until an operation runs.
        let store = S3Store::new(config).await.unwrap();
        assert_eq!(store.config().bucket().as_str(), "test-bucket");
        assert_eq!(store.config().region(), "us-east-1");
        assert!(store.config().path_style);
    }

    #[tokio::test]
    async fn s3_store_from_bucket_region_defaults_to_env_credentials() {
        let store =
            S3Store::from_bucket_region(BucketName::new("cfg-bucket").unwrap(), "eu-west-1")
                .await
                .unwrap();
        assert_eq!(store.config().bucket().as_str(), "cfg-bucket");
        assert_eq!(store.config().region(), "eu-west-1");
        assert!(matches!(
            store.config().credentials_mode,
            CredentialsMode::FromEnv
        ));
    }
}
