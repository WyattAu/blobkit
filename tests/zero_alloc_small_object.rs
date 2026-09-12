// Zero-allocation hot-path gate: a counting global allocator pins the
// allocation profile of the small-object fast path (`MemoryStore` put/get
// and `ObjectKey` validation) on every `cargo test` run.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Allocation counter tests for blobkit's small-object path.
//!
//! No per-op allocation claim existed for this path; this gate pins the
//! measured reality (2026-09-12):
//!
//! - warm-key `get` allocates **exactly 1/op — the `#[async_trait]` boxed
//!   future**. The payload path itself is allocation-free (stored data is
//!   a `bytes::Bytes`; delivery is a refcount bump, proven here).
//!   Migrating the trait to native AFIT would remove the box but costs
//!   dyn-compatibility — reported as future work, not a patch-release
//!   change;
//! - warm-key overwrite `put` is bounded ≤ 5/op (future box + metadata +
//!   fresh `BlobId` + map bookkeeping);
//! - `ObjectKey::new` allocates exactly the owned key `String`.
//!
//! A regression that sneaks a `format!`/`to_string()` into the warm get
//! path will fail the exact-count assertion. The iai-callgrind gate
//! (`benches/iai_hot_path.rs`; CI-only, requires valgrind) pins the
//! instruction cost of the same paths.

use std::alloc::{GlobalAlloc, Layout, System};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use blobkit::memory::MemoryStore;
use blobkit::{BlobStore, ObjectKey};
use bytes::Bytes;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

fn block_once<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("memory-store path must complete on its first poll"),
    }
}

const KEY: &str = "zero-alloc-probe.bin";
const PAYLOAD: &[u8] = b"small object payload";
const ITERATIONS: usize = 100;

/// One sequential test: the allocation counter is process-global, so
/// parallel test threads would pollute each other's counts.
#[test]
fn small_object_path_allocation_profile() {
    let store = MemoryStore::new();
    let key = ObjectKey::new(KEY).unwrap();

    // --- ObjectKey::new allocates exactly the owned key string. ---
    let before = allocations();
    let probe_key = ObjectKey::new("another-key.bin").unwrap();
    assert_eq!(
        allocations(),
        before + 1,
        "ObjectKey::new = exactly one allocation (the owned key String)"
    );

    // --- Warm the key (first put inserts the map entry). ---
    block_once(store.put(key.clone(), Bytes::from_static(PAYLOAD))).unwrap();
    block_once(store.get(&key)).unwrap();

    // --- Warm-key get: exactly 1 allocation per op — the `#[async_trait]`
    // boxed future. The payload path itself is allocation-free: the stored
    // data is a `bytes::Bytes`, and delivering it is a refcount bump
    // (proven below by cloning the returned handle).
    // Future work: migrating `BlobStore` to native AFIT (async-fn-in-trait)
    // would remove the per-call box but costs dyn-compatibility — a
    // breaking change, deliberately not done in a patch release. ---
    let before = allocations();
    for _ in 0..ITERATIONS {
        let data = block_once(store.get(&key)).unwrap();
        assert_eq!(&data[..], PAYLOAD);
        // Refcount bump on the payload: no allocation.
        let _handle = data.clone();
    }
    assert_eq!(
        allocations(),
        before + ITERATIONS,
        "warm-key get = exactly 1/op (the async_trait future box; the payload \
         is a refcount bump): got {} over {ITERATIONS}",
        allocations() - before
    );

    // --- Warm-key overwrite put: small, stable, bounded count. ---
    // Measured 2026-09-12: 1 (the async_trait future box) + metadata
    // (key/mime strings) + BlobId + map bookkeeping per overwrite;
    // bounded well below 5/op.
    let before = allocations();
    for _ in 0..ITERATIONS {
        block_once(store.put(key.clone(), Bytes::from_static(PAYLOAD))).unwrap();
    }
    let per_op = allocations() - before;
    assert!(
        per_op <= 5 * ITERATIONS,
        "overwrite put allocates ≤ 5/op (future box + metadata + id + map \
         bookkeeping); got {per_op} over {ITERATIONS} — a regression slipped in"
    );

    // --- Delivered bytes: caller dropping the payload does not count; a
    // cloned key passed to put is the caller's allocation (1 per op:
    // key.clone() below), asserting the *crate-side* overhead stays low.
    let before = allocations();
    let cloned = key.clone(); // caller-side key copy (1 alloc, not ours)
    assert_eq!(allocations(), before + 1);
    let before = allocations();
    block_once(store.put(cloned, Bytes::from_static(PAYLOAD))).unwrap();
    assert!(
        allocations() - before <= 4,
        "crate-side put overhead ≤ 4 allocations/op; got {}",
        allocations() - before
    );

    std::hint::black_box(probe_key);
}

// Sanity guard: if this fails, the assertions above prove nothing (the
// counter would be broken, not the path miraculously free).
#[test]
fn allocation_counter_sanity() {
    let before = allocations();
    std::hint::black_box(format!("fresh-{before}"));
    assert!(
        allocations() > before,
        "format! must allocate (counter sanity check)"
    );
}
