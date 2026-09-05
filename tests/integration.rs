// Tests exercise failure paths directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Integration tests exercising both backends through the unified `BlobStore` trait.

use blobkit::local::LocalStore;
use blobkit::memory::MemoryStore;
use blobkit::store::BlobStore;
use blobkit::types::ObjectKey;
use bytes::Bytes;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_put_get_roundtrip() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("integration/hello.txt").unwrap();
    let payload = Bytes::from("hello integration");
    store.put(key.clone(), payload.clone()).await.unwrap();
    let got = store.get(&key).await.unwrap();
    assert_eq!(got, payload);
}

#[tokio::test]
async fn memory_exists_delete() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("exists/del.txt").unwrap();
    assert!(!store.exists(&key).await.unwrap());
    store.put(key.clone(), Bytes::from("x")).await.unwrap();
    assert!(store.exists(&key).await.unwrap());
    store.delete(&key).await.unwrap();
    assert!(!store.exists(&key).await.unwrap());
    assert!(store.get(&key).await.is_err());
}

#[tokio::test]
async fn memory_overwrite_is_idempotent() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("overwrite.txt").unwrap();
    store.put(key.clone(), Bytes::from("v1")).await.unwrap();
    store.put(key.clone(), Bytes::from("v2")).await.unwrap();
    assert_eq!(store.get(&key).await.unwrap(), Bytes::from("v2"));
}

#[tokio::test]
async fn memory_presigned_url_unsupported() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("a.txt").unwrap();
    let res = store.presigned_url(&key, Duration::from_secs(60)).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn memory_empty_key_rejected() {
    let err = ObjectKey::new("").unwrap_err();
    assert!(err.is_invalid_key());
}

// ---------------------------------------------------------------------------
// Local
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_put_get_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    let key = ObjectKey::new("local/hello.txt").unwrap();
    let payload = Bytes::from("local hello");
    store.put(key.clone(), payload.clone()).await.unwrap();
    assert_eq!(store.get(&key).await.unwrap(), payload);
}

#[tokio::test]
async fn local_nested_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    let key = ObjectKey::new("a/b/c/d.bin").unwrap();
    store
        .put(key.clone(), Bytes::from(vec![0u8; 1024]))
        .await
        .unwrap();
    assert!(store.exists(&key).await.unwrap());
    store.delete(&key).await.unwrap();
    assert!(!store.exists(&key).await.unwrap());
}

#[tokio::test]
async fn local_max_bytes_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::with_limits(dir.path().to_path_buf(), Some(10))
        .await
        .unwrap();
    let key = ObjectKey::new("big.bin").unwrap();
    let err = store
        .put(key.clone(), Bytes::from(vec![0u8; 11]))
        .await
        .unwrap_err();
    assert!(matches!(err, blobkit::error::BlobError::StorageFull));
}

#[tokio::test]
async fn local_presigned_url_flow() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).await.unwrap();
    let key = ObjectKey::new("presign.txt").unwrap();
    // Not found before put
    assert!(store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .is_err());
    store.put(key.clone(), Bytes::from("data")).await.unwrap();
    let url = store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap();
    // URL should contain expiry
    #[cfg(feature = "s3")]
    assert!(url.to_string().contains("expires"));
    #[cfg(not(feature = "s3"))]
    assert!(url.contains("expires"));
}

#[tokio::test]
async fn local_and_memory_same_trait_object() {
    // Demonstrate that both backends can be used via `dyn BlobStore`.
    let mem: Box<dyn BlobStore> = Box::new(MemoryStore::new());
    let dir = tempfile::tempdir().unwrap();
    let local: Box<dyn BlobStore> =
        Box::new(LocalStore::new(dir.path().to_path_buf()).await.unwrap());

    for store in [&mem, &local] {
        let key = ObjectKey::new("dyn/test.txt").unwrap();
        store
            .put(key.clone(), Bytes::from("dyn hello"))
            .await
            .unwrap();
        assert_eq!(store.get(&key).await.unwrap(), Bytes::from("dyn hello"));
    }
}

#[cfg(not(feature = "s3"))]
#[tokio::test]
async fn s3_stub_returns_unsupported() {
    use blobkit::s3::{S3Config, S3Store};
    use blobkit::types::BucketName;
    let cfg = S3Config::new(BucketName::new("my-bucket").unwrap(), "us-east-1");
    let store = S3Store::new(cfg).await.unwrap();
    let key = ObjectKey::new("stub.txt").unwrap();
    let err = store.put(key.clone(), Bytes::from("hi")).await.unwrap_err();
    assert!(matches!(err, blobkit::error::BlobError::Unsupported(_)));
    let err = store.get(&key).await.unwrap_err();
    assert!(matches!(err, blobkit::error::BlobError::Unsupported(_)));
}

// ---------------------------------------------------------------------------
// WS-8 gap tests (REQ-BK-003/005/006/103/105/200/202)
// ---------------------------------------------------------------------------

/// REQ-BK-006: memory `put` must record metadata whose `sha256` digest
/// matches the stored bytes (content-integrity anchor).
#[cfg(all(feature = "memory", feature = "std", feature = "sha2"))]
#[tokio::test]
async fn memory_put_records_matching_sha256_metadata() {
    use blobkit::types::compute_sha256;

    let store = MemoryStore::new();
    let key = ObjectKey::new("integrity/digest.bin").unwrap();
    let payload = Bytes::from_static(b"digest me please");
    store.put(key.clone(), payload).await.unwrap();

    let meta = store.metadata(&key).expect("metadata must be recorded");
    assert_eq!(
        meta.sha256.as_deref(),
        Some(compute_sha256(b"digest me please").as_str())
    );
    assert_eq!(meta.size, 16);
}

/// REQ-BK-005: `compute_sha256` matches published SHA-256 vectors
/// (FIPS 180-4 empty-string vector included).
#[cfg(feature = "sha2")]
#[test]
fn compute_sha256_known_vector() {
    use blobkit::types::compute_sha256;

    assert_eq!(
        compute_sha256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        compute_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// REQ-BK-103: deleting a missing key must return `Err`, never panic.
#[cfg(feature = "memory")]
#[tokio::test]
async fn memory_delete_missing_key_is_err() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("never/written.txt").unwrap();
    let err = store.delete(&key).await.unwrap_err();
    assert!(matches!(err, blobkit::error::BlobError::NotFound(_)));
}

/// REQ-BK-003 / REQ-BK-105: every `put` — including overwrites of identical
/// content — must yield a fresh `BlobId`.
#[cfg(feature = "memory")]
#[tokio::test]
async fn memory_fresh_blob_id_per_put() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("ids/fresh.txt").unwrap();
    let payload = Bytes::from("same content");

    let id1 = store.put(key.clone(), payload.clone()).await.unwrap();
    let id2 = store.put(key.clone(), payload).await.unwrap();
    assert_ne!(id1, id2, "blob ids must never be reused");
}

/// REQ-BK-202: zero-byte payloads must round-trip intact.
#[cfg(feature = "memory")]
#[tokio::test]
async fn memory_empty_payload_roundtrip() {
    let store = MemoryStore::new();
    let key = ObjectKey::new("empty/payload.bin").unwrap();
    store.put(key.clone(), Bytes::new()).await.unwrap();
    assert_eq!(store.get(&key).await.unwrap(), Bytes::new());

    #[cfg(feature = "sha2")]
    {
        let meta = store.metadata(&key).unwrap();
        assert_eq!(
            meta.sha256.as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }
}

/// REQ-BK-200: concurrent puts/gets through one shared `MemoryStore` handle
/// must stay consistent — no lost writes, no torn reads.
#[cfg(feature = "memory")]
#[tokio::test]
async fn memory_concurrent_put_get_consistency() {
    use std::sync::Arc;

    let store = Arc::new(MemoryStore::new());
    let mut handles = Vec::new();

    for i in 0..16u32 {
        let store = Arc::clone(&store);
        handles.push(tokio::spawn(async move {
            let key = ObjectKey::new(format!("concurrent/key-{i}.txt")).unwrap();
            let payload = Bytes::from(format!("payload-{i}"));
            store.put(key.clone(), payload.clone()).await.unwrap();
            // Own write visible; other writers' keys may or may not exist yet.
            assert_eq!(store.get(&key).await.unwrap(), payload);
            assert!(store.exists(&key).await.unwrap());
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(store.len(), 16);
}
