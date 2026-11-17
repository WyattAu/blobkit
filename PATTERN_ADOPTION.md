# PATTERN_ADOPTION

Relationship to the WyattAu pattern canon (`pattern-standards`),
machine-checked by `pattern-standards/scripts/adoption-check.py`.

| Pattern | Status | Evidence |
|---|---|---|
| T1 | adopted | src/types.rs:51 (ObjectKey::new and BucketName::new validate at construction, so invalid keys are unrepresentable downstream) |
| P4 | adopted | src/s3.rs:94 (manual Debug impl renders the static secret key as "[redacted]" so credentials never leak through diagnostics) |
| P50 | partial | src/memory.rs:1 (the in-memory store is the hermetic twin the test suite runs against) and tests/s3.rs:102 (a live S3-compatible roundtrip exists but is opt-in via #[ignore] plus environment config: parity with real S3 is hand-run, not CI-enforced) |
| P64 | adopted | tests/proptest.rs:21 (property: any key containing a '..' path component is rejected) |
| P72 | adopted | src/error.rs:10 (one BlobError envelope across all backends); src/s3.rs:211 maps S3 status and error codes onto it, src/local.rs:193 maps io::ErrorKind::NotFound |
| P116 | adopted | src/types.rs:95 (keys with '..' path components are rejected: no directory traversal out of the store root) and src/local.rs:157 (writes go through tempfile plus persist, so a crash cannot leave a torn object) |
| T28 | n/a | Object keys are caller-chosen names, not content hashes; content-addressed identity is cas-kit's contract in this estate, and checksums here (optional sha2 feature) are metadata, not addresses |
