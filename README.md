Unified blob storage; trait + Memory + Local + S3.
| `s3` | - | `S3Store` via `aws-sdk-s3` (also MinIO/LocalStack/R2 via custom endpoints). |
| `serde` | - | `Serialize`/`Deserialize` for key types. |
| `chrono` | - | `created_at` timestamps on metadata. |
| `typed-id` | - | UUID-backed `BlobId`. |
| `sha2` | - | Content checksums on metadata. |
| `mime_guess` | - | Content-type detection from object keys. |
| `tracing` | - | `trace`/`debug` spans for store operations. |
| `io-uring` | - | (Linux) `IoUringStore`; `LocalStore` variant using raw io_uring SQEs for the bulk data path; see [PERF-SLO.md](PERF-SLO.md). |
| `object-store` | - | Additional backends via the `object_store` crate (GCP, Azure). |
Every numeric claim is mapped to its proof artifact in [CLAIMS.md](CLAIMS.md), which also pins the small-object fast path: `ObjectKey` validation = 490 instructions, warm `MemoryStore` get = 1 837 (iai-callgrind gate, `benches/iai_hot_path.rs`), and the allocation profile (get = exactly 1/op; the `#[async_trait]` future box; the `Bytes` payload is a refcount bump) proven by `tests/zero_alloc_small_object.rs`.