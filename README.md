# blobkit

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

- `std` (default), `memory`, `s3`, `typed-id`, `sha2`, `chrono`, `serde`, `tracing`
- `io-uring` (Linux only): `IoUringStore` — `LocalStore` variant using raw
  io_uring SQEs for the bulk data path; see [PERF-SLO.md](PERF-SLO.md).

## Security

Threat model: [THREAT-MODEL.md](THREAT-MODEL.md).
