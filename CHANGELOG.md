- `tests/config_matrix.rs`; per-knob behavior matrix completing the
- `benches/iai_hot_path.rs`; iai-callgrind instruction gate for the
- `tests/zero_alloc_small_object.rs`; counting-allocator gate pinning the
  `#[async_trait]` boxed future; the `Bytes` payload itself is a
  bulk data path uses raw io_uring SQEs; chunked, batched submit → wait →
    `IoUringFile` API stays synchronous (documented; intended for dedicated
    (`benches/io_uring_bench.rs`); io_uring ~1.4–1.6× faster than
## [0.2.1]: 2026-09-04
- Doc examples use `?` instead of `unwrap()`; zero `unwrap()` calls
## [0.2.0]: 2026-09-03
- `S3Config::with_path_style(bool)`; path-style addressing required by
- `S3Config::with_credentials(CredentialsMode)`; `FromEnv` (default),
- `S3Config::with_timeout(Option<Duration>)`; per-operation timeout
- `tests/s3.rs`; offline config tests plus an `#[ignore]`d live round-trip
## [0.1.0]: initial release