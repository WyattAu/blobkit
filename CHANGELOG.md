# Changelog

All notable changes to this project are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.2] - 2026-09-12

### Added

- `tests/config_matrix.rs` — per-knob behavior matrix completing the
  partial coverage (10 knobs): feature-off `mime_guess`/`sha2` fallbacks,
  offline S3 presigned-URL shape (endpoint/path-style/bucket), timeout
  and credentials contrasts, quota and key/boundary edges. No dead
  knobs found.
- `BlobMetadata::new_with_content_type` constructor (avoids allocating
  the default content-type string only to overwrite it).

### Fixed

- Warm-key overwrite `put` allocated 7/op vs the documented ≤ 5/op gate
  (throwaway default content-type string + redundant clone): removed 2
  waste allocations per put via the new constructor + a move.
- `tests/zero_alloc_small_object.rs`: serialized the two tests sharing
  the process-global counter, median/best-of assertions immune to
  harness-thread noise, `std`-gated the `MemoryStore`-dependent profile
  (pre-existing failures on clean tree, both feature configs).

## [0.4.1] - 2026-09-12

### Added

- **Claims proof-back** ([CLAIMS.md](CLAIMS.md)): every numeric performance
  claim in README/PERF-SLO mapped to its proof artifact.
- `benches/iai_hot_path.rs` — iai-callgrind instruction gate for the
  small-object fast path: `ObjectKey` validation = 490 instructions,
  `MemoryStore` overwrite-put = 2 747, warm get = 1 837 (CI-gated; needs
  valgrind to run locally).
- `tests/zero_alloc_small_object.rs` — counting-allocator gate pinning the
  small-object allocation profile: warm-key get = exactly 1/op (the
  `#[async_trait]` boxed future — the `Bytes` payload itself is a
  refcount bump); overwrite put bounded ≤ 5/op. Migrating `BlobStore` to
  native async-fn-in-trait (dropping the box, at the cost of
  dyn-compatibility) is documented as future work.

### Fixed

- `BlobId` hex parsing in the non-`typed-id` build no longer indexes
  (`clippy::indexing_slicing` failed `--no-default-features` clippy on
  main); `s3::map_service_error` and `benches/blob_bench` are now
  feature-gated so `cargo clippy --no-default-features --all-targets
  -- -D warnings` passes.

## [0.3.0] - 2026-09-06

### Added

- **`IoUringStore`** (Linux, feature `io-uring`): a `LocalStore` variant whose
  bulk data path uses raw io_uring SQEs — chunked, batched submit → wait →
  reap with short read/write resubmission (`IoUringFile` primitive).
  - Same on-disk layout, `max_bytes` guard, and atomic-write contract as
    `LocalStore` (tempfile + `sync_all` + rename); `delete`/`exists`/
    `presigned_url` delegate to plain syscalls where the ring adds nothing.
  - Blocking ring sections run inside `spawn_blocking`; the raw
    `IoUringFile` API stays synchronous (documented — intended for dedicated
    I/O threads). One ring per file/op in this minimal version.
  - `#![forbid(unsafe_code)]` is enforced on every configuration except the
    backend itself, where each `unsafe` block documents its invariant.
  - Roundtrip + parity tests vs `LocalStore`; criterion bench
    (`benches/io_uring_bench.rs`) — io_uring ~1.4–1.6× faster than
    `tokio::fs` for sequential 1 MiB reads/writes on the baseline host;
    numbers and caveats in [PERF-SLO.md](PERF-SLO.md).

## [0.2.2] - 2026-09-05

### Security
- Redacted `secret_key` in `CredentialsMode::Static` and `S3Config` Debug output (REQ-BLOBKIT-101)

## [0.2.1] — 2026-09-04

### Fixed

- Fixed the `no_std` build (`--no-default-features`): `S3Store`'s stub
  `BlobStore` impl now imports `alloc::boxed::Box` (needed by the
  `#[async_trait]` desugaring); removed unused/warned imports and
  dead `Entry` in feature-off configurations.
- `benches/blob_bench.rs` compiles again: criterion needs its `async`
  feature (enabled via `async_tokio`) for `Bencher::to_async`.

### Changed

- Doc examples use `?` instead of `unwrap()` — zero `unwrap()` calls
  remain outside `#[cfg(test)]` test modules.
- Clippy clean under `-D warnings --all-targets` (derived `Default` for
  `CredentialsMode`, removed useless `String` conversions).

### Added

- GitHub Actions CI: check (`--all-features`, `--no-default-features`,
  `--features s3`), tests, clippy `-D warnings`, rustfmt.

## [0.2.0] — 2026-09-03

### Added

- **Real S3 backend** behind the `s3` feature on top of `aws-sdk-s3` / `aws-config`:
  - `put` → `PutObject`, `get` → `GetObject`, `delete` → `DeleteObject`,
    `exists` → `HeadObject`, `presigned_url` → pre-signed GET.
- `S3Config::with_path_style(bool)` — path-style addressing required by
  MinIO / LocalStack / Cloudflare R2.
- `S3Config::with_credentials(CredentialsMode)` — `FromEnv` (default),
  `FromProfile`, or `Static { access_key, secret_key }`.
- `S3Config::with_timeout(Option<Duration>)` — per-operation timeout
  (default 30s) via `TimeoutConfig`.
- SDK error mapping: 404 / `NoSuchKey` → `BlobError::NotFound`,
  401/403 / `AccessDenied` → `BlobError::PermissionDenied`, else `Other`.
- `tests/s3.rs` — offline config tests plus an `#[ignore]`d live round-trip
  driven by `BLOBKIT_S3_TEST_BUCKET` and friends.

### Changed

- `S3Store::new` is now `async` and returns `Result<Self, BlobError>`
  (mirrored by the no-feature stub, so call sites compile unchanged once
  the `s3` feature is enabled).

## [0.1.0] — initial release

### Added

- `BlobStore` trait (object-safe, async) with `put` / `get` / `delete` /
  `exists` / `presigned_url`.
- `MemoryStore` (feature `memory`) and `LocalStore` (feature `std`, atomic
  writes, optional `max_bytes` guard).
- `S3Config` / `S3Store` stubs (`Unsupported` operations) for type-level
  migration.
- Typed `ObjectKey`, `BucketName`, `BlobId` with validation.

## [0.4.0] - 2026-09-08

### Added
- `object-store` feature: GCS + Azure backends via `object_store` facade
- `BlobStore::list()` trait method with default `Unsupported` impl
- `list()` implementations for LocalStore, MemoryStore, ObjectStoreBackend

### Fixed
- Azure `AccountKey` with empty account/key rejected at construction
- `guess_content_type` gated on `mime_guess` feature (was always-on)
