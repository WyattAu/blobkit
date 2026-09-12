# Performance SLOs & Benchmarks

Every numeric claim in this file and the README is inventoried against its
proof artifact in [CLAIMS.md](CLAIMS.md).

Numbers from `benches/io_uring_bench.rs` (criterion, release profile). Re-run
with:

```sh
cargo bench --features io-uring --bench io_uring_bench
```

## Environment (baseline captured 2026-09-06)

- Linux 7.2.2-1-cachyos, io_uring enabled (kernel ≥ 5.1 required by the backend)
- Intel Core i5-9400F (6 cores), NVMe (non-rotational) root device
- blobkit 0.3.0, `io-uring` crate 0.7, default ring depth 64, 128 KiB chunks

## Sequential 1 MiB write/read (bulk data path, no fsync in timed section)

| Operation | `tokio::fs` | `IoUringFile` | io_uring speedup |
|-----------|------------:|--------------:|-----------------:|
| write 1 MiB | 1.74 ms (0.59 GiB/s) | 1.22 ms (0.84 GiB/s) | ~1.43× |
| read 1 MiB  | 2.56 ms (0.40 GiB/s) | 1.63 ms (0.63 GiB/s) | ~1.57× |

`BlobStore` end-to-end (includes tempfile create, `sync_all`, rename for
`put`; metadata + open for `get`):

| Operation | Median |
|-----------|-------:|
| `IoUringStore::put` 1 MiB | 2.50 ms (0.37 GiB/s) |
| `IoUringStore::get` 1 MiB | 1.33 ms (0.76 GiB/s) |

## SLOs

- **Raw path regression guard:** `IoUringFile` sequential 1 MiB write/read must
  not regress > 25% against the `tokio::fs` baseline in the same environment
  (i.e. must remain faster — currently 1.4–1.6×).
- **Durability is not traded for speed:** `put` always fsyncs before rename;
  end-to-end `put` ≈ raw write + 2× is expected and acceptable.
- The criterion baselines in `target/criterion` serve as the comparison
  point; re-run on matching hardware when judging regressions.

## Honest caveats

- The io_uring path includes per-operation ring setup (a handful of syscalls,
  ~µs) — the backend deliberately uses one ring per file/op in this minimal
  version. Amortizing a ring per worker thread is future work.
- Neither side fsyncs inside the timed write sections: this measures bulk data
  transfer, not durability overhead. The *store* benchmarks include the full
  atomic-write contract.
- Single-process, single-threaded-iteration, page-cache-warm numbers on one
  desktop NVMe host. Treat as relative (A vs B on the same box), not absolute.
- The `BlobStore` methods run the ring in `spawn_blocking`; a dedicated I/O
  thread or fully async integration would shift the numbers.
