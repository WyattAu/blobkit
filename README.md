# blobkit

Unified blob storage — trait + Memory + Local + S3 stub.

Replaces `S3Client::from_conf` duplication across `archival-shim`, `backup-shim`, `kestrel-storage`, and `ferro`.

```rust
use blobkit::memory::MemoryStore;
use blobkit::store::BlobStore;
use blobkit::types::ObjectKey;
use bytes::Bytes;

# #[tokio::main]
# async fn main() -> Result<(), blobkit::error::BlobError> {
let store = MemoryStore::new();
let key = ObjectKey::new("hello.txt").unwrap();
store.put(key.clone(), Bytes::from("hello world")).await?;
let data = store.get(&key).await?;
assert_eq!(data, Bytes::from("hello world"));
# Ok(())
# }
```

## Backends

| Backend | Feature | Durable |
|---------|---------|---------|
| `MemoryStore` | `memory` | No |
| `LocalStore` | `std` | Yes |
| `S3Store` | `s3` | Yes (stub in v0.1) |

## Features

- `std` (default), `memory`, `s3`, `typed-id`, `sha2`, `chrono`, `serde`, `tracing`
