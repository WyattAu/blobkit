# Requirements: blobkit (integrity-relevant subset)
Scope: the integrity-relevant surface; `ObjectKey`/`BucketName` validation
| REQ-BK-003 | `memory_overwrite_is_idempotent` (`tests/integration.rs`); `memory_fresh_blob_id_per_put`; **gap test added** | integration |
| REQ-BK-005 | `compute_sha256_known_vector` (`tests/integration.rs`); **gap test added** | unit |
| REQ-BK-006 | `memory_put_records_matching_sha256_metadata` (`tests/integration.rs`); **gap test added** (requires `sha2` feature) | integration |
| REQ-BK-103 | `memory_exists_delete` (get-after-delete); `memory_delete_missing_key_is_err` (`tests/integration.rs`); **gap test added** | integration |
| REQ-BK-105 | `memory_fresh_blob_id_per_put` (`tests/integration.rs`); **gap test added** | integration |
| REQ-BK-200 | `memory_concurrent_put_get_consistency` (`tests/integration.rs`); **gap test added** | concurrency |
| REQ-BK-202 | `memory_empty_payload_roundtrip` (`tests/integration.rs`); **gap test added** | integration |