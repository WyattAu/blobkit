# Changelog

All notable changes to this project are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
