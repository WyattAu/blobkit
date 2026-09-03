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
