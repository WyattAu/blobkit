# blobkit

[![docs.rs](https://docs.rs/blobkit/badge.svg)](https://docs.rs/blobkit)
[![crates.io](https://img.shields.io/crates/v/blobkit.svg)](https://crates.io/crates/blobkit)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

Unified blob storage — trait + Memory + Local + S3.

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
| `S3Store` | `s3` | Yes (aws-sdk-s3) |

## S3 backend

Enabled by `features = ["s3"]` (pulls in `aws-sdk-s3` + `aws-config`):

```rust,no_run
use blobkit::s3::{CredentialsMode, S3Config, S3Store};
use blobkit::types::BucketName;
use url::Url;

# async fn example() -> Result<(), blobkit::error::BlobError> {
let cfg = S3Config::new(BucketName::new("my-bucket")?, "us-east-1")
    .with_endpoint(Url::parse("http://localhost:9000")?) // MinIO/LocalStack/R2
    .with_path_style(true)                               // required by most local stores
    .with_credentials(CredentialsMode::Static {
        access_key: "minioadmin".into(),
        secret_key: "minioadmin".into(),
    });
let store = S3Store::new(cfg).await?;
# Ok(())
# }
```

- Credentials: `CredentialsMode::FromEnv` (default), `FromProfile`, or `Static`.
- Operations map to S3 `PutObject` / `GetObject` / `DeleteObject` / `HeadObject`;
  `presigned_url` generates a pre-signed GET.
- 404 / `NoSuchKey` → `BlobError::NotFound`; 401/403 / `AccessDenied` → `BlobError::PermissionDenied`.
- Without the `s3` feature the module stays a light stub (all ops return `Unsupported`).

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | ✅ | `LocalStore` filesystem backend. |
| `memory` | ✅ | `MemoryStore` in-memory backend. |
| `s3` | — | `S3Store` via `aws-sdk-s3` (also MinIO/LocalStack/R2 via custom endpoints). |
| `serde` | — | `Serialize`/`Deserialize` for key types. |
| `chrono` | — | `created_at` timestamps on metadata. |
| `typed-id` | — | UUID-backed `BlobId`. |
| `sha2` | — | Content checksums on metadata. |
| `mime_guess` | — | Content-type detection from object keys. |
| `tracing` | — | `trace`/`debug` spans for store operations. |
| `io-uring` | — | (Linux) `IoUringStore` — `LocalStore` variant using raw io_uring SQEs for the bulk data path; see [PERF-SLO.md](PERF-SLO.md). |
| `object-store` | — | Additional backends via the `object_store` crate (GCP, Azure). |

## Performance

Measured io_uring bulk-data path (criterion, 2026-09, 6-core x86_64 NVMe; sequential 1 MiB, no fsync in timed section):

| Operation | `tokio::fs` | `IoUringFile` | Speedup |
|-----------|------------:|--------------:|-----------------:|
| write 1 MiB | 1.74 ms | 1.22 ms | ~1.43× |
| read 1 MiB | 2.56 ms | 1.63 ms | ~1.57× |

End-to-end `BlobStore`: `IoUringStore::put` 1 MiB ≈ 2.50 ms, `get` ≈ 1.33 ms (includes the full atomic-write/durability contract). Full SLO policy and honest caveats: [PERF-SLO.md](PERF-SLO.md).

Every numeric claim is mapped to its proof artifact in [CLAIMS.md](CLAIMS.md), which also pins the small-object fast path: `ObjectKey` validation = 490 instructions, warm `MemoryStore` get = 1 837 (iai-callgrind gate, `benches/iai_hot_path.rs`), and the allocation profile (get = exactly 1/op — the `#[async_trait]` future box; the `Bytes` payload is a refcount bump) proven by `tests/zero_alloc_small_object.rs`.

## Security

Threat model: [THREAT-MODEL.md](THREAT-MODEL.md).
