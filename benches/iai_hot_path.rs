// iai-callgrind benchmarks run once under Valgrind on fixed inputs; the
// harness measures instruction counts, so there is no "expected failure"
// recovery path — a panic aborts the run visibly, which is what we want.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Deterministic gate for the small-object fast path: `MemoryStore`
//! put/get (the in-process backend) plus `ObjectKey` validation.
//!
//! The io_uring bulk path (PERF-SLO.md, ~1.4–1.6× vs `tokio::fs`) is
//! wall-clock-benched in `benches/io_uring_bench.rs` — its cost lives in
//! NVMe syscalls and DMA, where instruction counts are not the interesting
//! metric and background kernel state would make them wobble. This file
//! pins the CPU-only fast path that every small object pays on the way
//! through any backend-agnostic caller.
//!
//! Locally requires `valgrind`; without it, compile-check only:
//! `cargo bench --no-run --bench iai_hot_path`.

use std::future::Future;
use std::hint::black_box;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use blobkit::memory::MemoryStore;
use blobkit::{BlobStore, ObjectKey};
use bytes::Bytes;
use iai_callgrind::{library_benchmark, library_benchmark_group, main};

/// Poll a ready-on-first-poll future without a runtime: the MemoryStore
/// paths are straight-line code over a synchronized map, so the noop-waker
/// poll measures exactly the crate's path — no tokio plumbing in the
/// count.
fn block_once<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("memory-store path must complete on its first poll"),
    }
}

const KEY: &str = "iai-hot-path.bin";
const PAYLOAD: &[u8] = b"iai small-object payload";

fn setup_store() -> (MemoryStore, ObjectKey) {
    let store = MemoryStore::new();
    let key = ObjectKey::new(KEY).unwrap();
    block_once(store.put(key.clone(), Bytes::from_static(PAYLOAD))).unwrap();
    (store, key)
}

// Key validation: every put/get caller pays this.
#[library_benchmark]
#[bench::valid_key(KEY)]
fn object_key_validate(key: &str) -> Result<ObjectKey, blobkit::BlobError> {
    black_box(ObjectKey::new(key))
}

// Overwrite-put of a small object on a warm key (map entry exists).
#[library_benchmark]
#[bench::steady_state(setup = setup_store)]
fn memory_put_small(env: (MemoryStore, ObjectKey)) -> Result<blobkit::BlobId, blobkit::BlobError> {
    let (store, key) = env;
    black_box(block_once(store.put(key, Bytes::from_static(PAYLOAD))))
}

// Get of a small object on a warm key.
#[library_benchmark]
#[bench::steady_state(setup = setup_store)]
fn memory_get_small(env: (MemoryStore, ObjectKey)) -> Option<Bytes> {
    let (store, key) = env;
    black_box(block_once(store.get(&key)).ok())
}

library_benchmark_group!(
    name = iai_hot_path;
    benchmarks =
        object_key_validate,
        memory_put_small,
        memory_get_small
);

main!(library_benchmark_groups = iai_hot_path);
