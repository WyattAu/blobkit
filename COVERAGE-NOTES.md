# Coverage notes — blobkit

Measurement: `cargo llvm-cov --summary-only --all-features` (line coverage, llvm-cov 0.8.7).
Current: **92.9%** (Sept 2026 hard-gate prep), threshold 90% for Tier A — PASS.

## Accepted exceptions

Lines below are not covered by hermetic tests. Each is a deliberate exception
with rationale; they are excluded from the effective coverage judgment for the
October hard gate.

### s3.rs — live-endpoint operations (~33 lines)

`BlobStore for S3Store` (`put` / `get` / `delete` / `exists` / `presigned_url`)
and the SDK error mapper (`map_sdk_error` and its status/code branches) execute
only when a request reaches a real S3-compatible endpoint. Everything up to and
including client construction (static credentials, region, endpoint, path-style,
timeout) is covered by `tests/coverage_gates.rs` (offline, no network).

Live round-trip tests exist but are `#[ignore]`-gated: `tests/s3.rs`
(`live_roundtrip`, requires a live S3-compatible endpoint; see that file's
module docs). Rationale: hermetic tests cannot fabricate aws-sdk HTTP
responses without a mock server, which is out of scope for this crate's
test budget; the ignored live test is the intended vehicle.

### io_uring_backend.rs — kernel completion-loop paths (~18 lines)

- Short-read / short-write resubmission and the zero-length-completion error
  path inside the ring completion loop: these require the kernel to complete an
  SQE partially or with 0 bytes, which cannot be injected deterministically
  from userspace (no fault-injection hooks around `io_uring`).
- `spawn_blocking` join-error mapping (`blocking put` / `blocking get`):
  requires the tokio runtime to be shutting down mid-operation.
- `with_limits` "not a directory" branch: `create_dir_all` fails first with
  `EEXIST` when the root exists as a file, so this defensive branch is
  unreachable through the public API (covered by a test asserting the actual
  `BlobError::Io` behavior instead).
- `tempfile create` / `persist` error mappings: require injected filesystem
  failures (unwritable parent, EISDIR rename targets).

### local.rs — injected-filesystem-failure paths (~6 lines)

- "not a directory" branch in `with_limits` (same EEXIST pre-emption as above).
- tempfile write/flush/sync error mappings (lines ~160-170): require a disk
  full / I/O error mid-write; not injectable hermetically.

### Feature-cfg union artifacts (~7 lines)

Measured with `--all-features`, cfg-negation blocks compile only when a
feature is *disabled* and can never execute in this configuration:

- `memory.rs` `#[cfg(not(feature = "std"))]` / `#[cfg(all(not(feature =
  "memory"), feature = "std"))]` fallback branches in `list_keys`.
- `types.rs` `#[cfg(not(feature = "typed-id"))] impl Default for BlobId`.
- `store.rs` `#[cfg(not(feature = "s3"))] presigned_url` forward.

A per-feature-matrix measurement (e.g. `--features std,memory` and
`--features std` runs) would cover these; not worth the CI cost for ~7 lines.
