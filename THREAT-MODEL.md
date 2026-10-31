# Threat Model: blobkit
| T2 | Control-character injection in keys | Tampering | `ObjectKey::validate` | `\0`, `\r`, `\n` rejected at construction | `tests/proptest.rs` (proptest strategies are `\\PC*`; no control chars survive validation); `tests/integration.rs::memory_empty_key_rejected` |
## OPEN RISKS (missing mitigations: not fabricated)
- **OPEN-1; `max_bytes` is opt-in and `None` by default.**
- **OPEN-2; `ObjectKey::new_unchecked` bypasses validation.** Documented
- **OPEN-3; no symlink hardening on `LocalStore`.** A key path that an
- **OPEN-4; `S3Config` derives `Debug` with `CredentialsMode::Static`
- **OPEN-5; no differential test that the *same* hostile key is rejected
  accepted (contain no `..` *component*); safe for path purposes, but
  (~512 segments); disk-inode pressure is a resource concern, not a