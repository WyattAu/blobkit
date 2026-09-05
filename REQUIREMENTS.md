# Requirements — blobkit (integrity-relevant subset)

Numbered, testable requirements. Every requirement maps to at least one named
test; every security-relevant test cites at least one requirement. Doc
comments on the implementing public item carry `REQ-BK-NNN` tags.

Scope: the integrity-relevant surface — `ObjectKey`/`BucketName` validation
(injection/traversal defense), `BlobStore` put/get/delete/exists semantics,
content digesting (`compute_sha256`, metadata `sha256`), and the memory
backend's concurrency behavior. S3 wire details are out of scope for WS-8.

## Functional

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-BK-001 | `put` followed by `get` returns byte-identical data (memory and local backends) | MUST |
| REQ-BK-002 | `exists`/`delete` lifecycle: absent → `false`; after put → `true`; after delete → `false` and `get` → `Err(NotFound)` | MUST |
| REQ-BK-003 | Overwrite semantics: last write wins and each `put` (including overwrites) yields a fresh `BlobId` | MUST |
| REQ-BK-004 | `presigned_url`: backends without support return `Err(Unsupported)`; the local backend produces a usable URL before expiry | SHOULD |
| REQ-BK-005 | `compute_sha256` returns the correct lowercase hex SHA-256 digest (known-vector checked, empty input included) | MUST |
| REQ-BK-006 | `MemoryStore::put` records metadata whose `sha256` equals the digest of the stored bytes, with correct size | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-BK-100 | `ObjectKey::new` rejects any key containing a `..` path component (directory-traversal defense), for arbitrary hostile strings | MUST |
| REQ-BK-101 | `ObjectKey::new` enforces bounds: non-empty, ≤ 1024 bytes, no null/CR/LF, no empty path segments or leading/trailing slashes | MUST |
| REQ-BK-102 | `BucketName::new` enforces 3–63 chars, lowercase alphanumeric + hyphen only, alphanumeric start/end | MUST |
| REQ-BK-103 | `get`/`delete` of a missing key returns `Err`, never panics (missing key is an error, not a crash) | MUST |
| REQ-BK-104 | The local backend enforces the configured `max_bytes` quota and rejects writes that exceed it | MUST |
| REQ-BK-105 | `BlobId`s are unique across puts (no id reuse, even for identical content) | MUST |
| REQ-BK-106 | Validation happens at the type boundary (`ObjectKey::new`) before any I/O; `new_unchecked` is the explicit, documented escape hatch for trusted sources | SHOULD |

## Robustness

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-BK-200 | A `MemoryStore` handle shared across concurrent tasks maintains put/get consistency (no lost or torn writes) | MUST |
| REQ-BK-201 | `BlobStore` is object-safe; `Box<dyn BlobStore>` forwards all operations correctly | MUST |
| REQ-BK-202 | Zero-byte payloads round-trip intact | SHOULD |

## Constant-Time Audit

- AUDIT: this crate holds no secrets and performs no secret-dependent
  comparisons. The only digests (`compute_sha256`, `src/types.rs`) address
  public blob content; equality checks on digests/ids compare public data.
  ✓ No `subtle`/CT requirement applies.

## Traceability Matrix

| Requirement | Test (fn, file) | Property class |
|-------------|-----------------|----------------|
| REQ-BK-001 | `memory_put_get_roundtrip`, `local_put_get_roundtrip` (`tests/integration.rs`) | integration |
| REQ-BK-002 | `memory_exists_delete` (`tests/integration.rs`) | integration |
| REQ-BK-003 | `memory_overwrite_is_idempotent` (`tests/integration.rs`); `memory_fresh_blob_id_per_put` — **gap test added** | integration |
| REQ-BK-004 | `memory_presigned_url_unsupported`, `local_presigned_url_flow` (`tests/integration.rs`) | integration |
| REQ-BK-005 | `compute_sha256_known_vector` (`tests/integration.rs`) — **gap test added** | unit |
| REQ-BK-006 | `memory_put_records_matching_sha256_metadata` (`tests/integration.rs`) — **gap test added** (requires `sha2` feature) | integration |
| REQ-BK-100 | `object_key_rejects_dotdot` (`src/types.rs`); `object_key_rejects_dotdot_component` (proptest, `tests/proptest.rs`) | unit/fuzz |
| REQ-BK-101 | `object_key_rejects_empty`, `object_key_rejects_too_long`, `object_key_rejects_control_chars`, `object_key_rejects_empty_segments` (`src/types.rs`); `object_key_max_len_enforced`, `object_key_rejects_empty_segments` (proptests) | unit/fuzz |
| REQ-BK-102 | `bucket_name_valid`, `bucket_name_invalid` (`src/types.rs`); `bucket_name_valid_pattern` (proptest) | unit/fuzz |
| REQ-BK-103 | `memory_exists_delete` (get-after-delete); `memory_delete_missing_key_is_err` (`tests/integration.rs`) — **gap test added** | integration |
| REQ-BK-104 | `local_max_bytes_enforced` (`tests/integration.rs`) | integration |
| REQ-BK-105 | `memory_fresh_blob_id_per_put` (`tests/integration.rs`) — **gap test added** | integration |
| REQ-BK-106 | Structural: `new_unchecked` doc (`src/types.rs`) + negative tests under REQ-BK-100/101 | design |
| REQ-BK-200 | `memory_concurrent_put_get_consistency` (`tests/integration.rs`) — **gap test added** | concurrency |
| REQ-BK-201 | `local_and_memory_same_trait_object` (`tests/integration.rs`) | integration |
| REQ-BK-202 | `memory_empty_payload_roundtrip` (`tests/integration.rs`) — **gap test added** | integration |

## Test Count Delta

- Before: 52 tests under `--all-features` (32 unit + 10 integration + 6 in `tests/proptest.rs` + 4 in `tests/s3.rs`, one ignored pending live S3).
- Added: 6 (`memory_put_records_matching_sha256_metadata`, `compute_sha256_known_vector`, `memory_delete_missing_key_is_err`, `memory_fresh_blob_id_per_put`, `memory_empty_payload_roundtrip`, `memory_concurrent_put_get_consistency`).
- After: 58.
