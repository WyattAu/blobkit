# Performance claims inventory — blobkit

Every performance claim in [README.md](README.md) and [PERF-SLO.md](PERF-SLO.md),
mapped to its proof artifact. Created 2026-09-12 (blobkit 0.4.1).

Machine context for wall-clock records: Linux 7.2.2-1-cachyos, i5-9400F
(6 cores), NVMe, io-uring crate 0.7, ring depth 64, 128 KiB chunks,
captured 2026-09-06 on blobkit 0.3.0. Proof kinds: **criterion** (wall-clock
record), **iai** (instruction gate, `cargo bench --bench iai_hot_path`),
**test**, **code**, **compiler**.

## io_uring bulk-data path (criterion, `benches/io_uring_bench.rs` — measured-on records)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 1 | write 1 MiB: `tokio::fs` 1.74 ms vs `IoUringFile` 1.22 ms, ~1.43× | `io_uring_bench` write arms | backed (measured on) |
| 2 | read 1 MiB: 2.56 ms vs 1.63 ms, ~1.57× | same, read arms | backed (measured on) |
| 3 | `IoUringStore::put` 1 MiB end-to-end: 2.50 ms (incl. fsync + rename) | same, store arms | backed (measured on) |
| 4 | `IoUringStore::get` 1 MiB end-to-end: 1.33 ms | same | backed (measured on) |
| 5 | SLO: raw path must not regress > 25 % vs `tokio::fs` in the same environment | policy; CI criterion baselines (`--save-baseline ci`) are the comparison point | backed (policy) |
| 6 | No fsync inside the timed write sections (bulk-transfer measurement, not durability) | bench source (`benches/io_uring_bench.rs`), documented methodology | backed (code) |
| 7 | io_uring path includes per-op ring setup (~µs, a few syscalls); one ring per file/op is deliberate in the minimal version | code (`src/io_uring_backend.rs`) + PERF-SLO caveat | backed (code) |

## Durability / correctness contract

| # | Claim | Artifact | Status |
|---|---|---|---|
| 8 | `put` always fsyncs before rename (durability not traded for speed; put ≈ raw write + 2× acceptable) | `src/local.rs` write path + `tests/integration.rs` atomic-write tests | **proven** (test) |
| 9 | Atomic-write contract: tempfile → fsync → rename | same tests | **proven** (test) |

## Small-object fast path (new gates, 0.4.1)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 10 | CPU fast path pinned: `ObjectKey::new` = **490 instructions**, `MemoryStore` overwrite-put = **2 747**, warm get = **1 837** (2026-09-12, valgrind 3.25.1) | `benches/iai_hot_path.rs` | **proven** (iai) |
| 11 | Allocation profile of the small-object path: warm-key get = **exactly 1/op — the `#[async_trait]` boxed future**; the payload path itself is allocation-free (`Bytes` refcount bump, proven by handle-clone); overwrite put bounded ≤ 5/op | `tests/zero_alloc_small_object.rs` (runs on every `cargo test`) | **proven** (test) |
| 12 | Future work: `BlobStore` could drop the per-call box via native async-fn-in-trait | noted in the test docs; costs dyn-compatibility — not a patch-release change | documented (future work) |

## Other

| # | Claim | Artifact | Status |
|---|---|---|---|
| 13 | `BlobStore` methods run the io_uring ring in `spawn_blocking` | code (`src/io_uring_backend.rs`) | backed (code) |
| 14 | io-uring needs kernel ≥ 5.1; crate 0.7, depth 64, 128 KiB chunks | `Cargo.toml` (`io-uring = "0.7"`) + backend constants | **proven** (manifest/code) |
| 15 | `BlobStore` methods run the io_uring ring in `spawn_blocking` — the `BlobStore` trait stays dyn-compatible via `#[async_trait]` (hence the 1-op box measured in claim 11) | code + alloc test | backed (code + test) |

## Totals

- **Proven by hard artifact (iai/test/manifest):** 6 (claims 8, 9, 10, 11, 14)
- **Backed (measured-on criterion records, policy, code reading):** 9
- **Deleted/reworded:** 0 (the small-object allocation claim did not exist
  before; it is now pinned by measurement, including the honest
  async_trait-box finding)

## Reproducing

```sh
cargo bench --features io-uring --bench io_uring_bench   # wall-clock A vs B
cargo bench --bench iai_hot_path                          # instruction gate (valgrind)
cargo test  --test zero_alloc_small_object                # allocation gate
cargo test  --all-features                                # durability/atomic-write tests
```
