# Threat Model — blobkit

Status: **v1.0** · Method: STRIDE over the public API surface
(`ObjectKey`/`BucketName` validation, `BlobStore` trait, `LocalStore`,
`MemoryStore`, `S3Store`).

Trust boundaries: (1) object keys and payloads arriving from callers
(frequently attacker-influenced: filenames, user-supplied keys), (2) the
filesystem root directory, (3) the S3 endpoint and credentials, (4) the
`object_store`/`tokio` dependency tree.

## Assets

| ID | Asset | Example |
|----|-------|---------|
| A1 | Filesystem containment | `../../etc/cron.d/x` written outside root |
| A2 | Stored data integrity | One tenant's payload overwritten via crafted key |
| A3 | Caller availability | Unbounded payload → OOM/disk exhaustion |
| A4 | S3 credentials | Static keys leaked via `Debug`/logs |

## STRIDE Analysis

| # | Threat | Category | Surface | Mitigation | Verifying test |
|---|--------|----------|---------|------------|----------------|
| T1 | Path traversal via `..` components | Elevation | `ObjectKey::new`, `LocalStore::resolve` | Validation rejects any `..` path component, empty segments, leading/trailing `/`, null/CR/LF, keys > 1024 bytes; `resolve` re-checks `..` as defense in depth | `tests/proptest.rs::object_key_rejects_dotdot_component`, `object_key_max_len_enforced`, `object_key_rejects_empty_segments`; `tests/integration.rs::local_nested_keys` |
| T2 | Control-character injection in keys | Tampering | `ObjectKey::validate` | `\0`, `\r`, `\n` rejected at construction | `tests/proptest.rs` (proptest strategies are `\\PC*` — no control chars survive validation); `tests/integration.rs::memory_empty_key_rejected` |
| T3 | Payload size bombs (OOM / disk exhaustion) | DoS | `LocalStore::put` | `max_bytes` rejects oversized payloads with `StorageFull` *before* touching the filesystem | `tests/integration.rs::local_max_bytes_enforced` |
| T4 | Partial/corrupt writes on crash | Tampering | `LocalStore::put` | Atomic writes: `tempfile` + `rename` | documented in `src/local.rs` module docs; exercised implicitly by `local_put_get_roundtrip` |
| T5 | Key/bucket name confusion across stores | Tampering | `ObjectKey`, `BucketName` | Identical validation rules for Memory/Local/S3; bucket names constrained to AWS rules (3–63, lowercase alnum + hyphen) | `tests/proptest.rs::bucket_name_valid_pattern`; `tests/integration.rs::memory_and_memory_same_trait_object` / `local_and_memory_same_trait_object` |
| T6 | Malformed payload handling panics | DoS | all stores | `#![forbid(unsafe_code)]`; errors are `Result`/`BlobError` | `tests/proptest.rs::memory_put_get_roundtrip_data` (arbitrary bytes 0..4096), `blob_id_roundtrip` |
| T7 | Presigned-URL expectations leak into unsupported backends | Repudiation | `MemoryStore::presigned` | Returns explicit `unsupported` instead of silently succeeding | `tests/integration.rs::memory_presigned_url_unsupported`, `s3_stub_returns_unsupported` |

## OPEN RISKS (missing mitigations — not fabricated)

- **OPEN-1 — `max_bytes` is opt-in and `None` by default.**
  `LocalStore::new` creates an unlimited store; only `with_limits`/
  `with_max_bytes` enable the T3 guard. A caller using the plain
  constructor has no size-bomb protection.
- **OPEN-2 — `ObjectKey::new_unchecked` bypasses validation.** Documented
  escape hatch ("trusted source"). `LocalStore::resolve` still rejects `..`,
  but `MemoryStore`/`S3Store` accept whatever an unchecked key carries.
- **OPEN-3 — no symlink hardening on `LocalStore`.** A key path that an
  attacker could point at a pre-planted symlink inside the root would be
  followed on write; local-FS adversary is declared out of scope, but
  O_NOFOLLOW-style defenses (cf. kestrel) are absent.
- **OPEN-4 — `S3Config` derives `Debug` with `CredentialsMode::Static`
  containing `secret_key`.** A `{:?}` of the config prints the secret
  (same class of issue as tokenkit OPEN-1). No redacted `Debug`.
- **OPEN-5 — no differential test that the *same* hostile key is rejected
  identically by all three backends** (validation is centralized in
  `ObjectKey`, but per-store guard tests only cover Memory/Local).

## Out of Scope

- S3-side policies (bucket ACLs, encryption at rest, IAM scoping).
- Concurrent same-key write races beyond atomicity of single-object rename.
- Malicious S3 endpoint (configured by the deployer, not the attacker).

## Residual Risks

- Key validation is substring/segment based: keys like `a..b` or `....` are
  accepted (contain no `..` *component*) — safe for path purposes, but
  callers using keys as display identifiers see odd strings.
- 1024-byte key cap matches S3, so LocalStore allows deeply nested trees
  (~512 segments) — disk-inode pressure is a resource concern, not a
  traversal one.
