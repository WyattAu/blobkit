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
#[cfg(feature = "std")]
use std::future::Future;
#[cfg(feature = "std")]
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "std")]
use std::task::{Context, Poll, Waker};

#[cfg(feature = "std")]
use blobkit::memory::MemoryStore;
#[cfg(feature = "std")]
use blobkit::{BlobStore, ObjectKey};
#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
fn block_once<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("memory-store path must complete on its first poll"),
    }
}

#[cfg(feature = "std")]
const KEY: &str = "zero-alloc-probe.bin";
#[cfg(feature = "std")]
const PAYLOAD: &[u8] = b"small object payload";
#[cfg(feature = "std")]
const ITERATIONS: usize = 100;

/// The allocation counter is process-global and the harness runs tests in
/// parallel threads: serialize the two tests so the sanity check's
/// allocations can never pollute the profile's measured regions.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One sequential test: the allocation counter is process-global, so
/// parallel test threads would pollute each other's counts.
///
/// Requires `std`: `MemoryStore` is a stub without it.
#[cfg(feature = "std")]
#[test]
fn small_object_path_allocation_profile() {
    let _guard = SERIAL.lock().unwrap();
    let store = MemoryStore::new();
    let key = ObjectKey::new(KEY).unwrap();

    // --- ObjectKey::new allocates exactly the owned key string. ---
    // Best-of-3: background threads can only ADD counts, so the minimum is
    // the exact cost.
    let mut best = usize::MAX;
    for _ in 0..3 {
        let before = allocations();
        let probe = ObjectKey::new("another-key.bin").unwrap();
        best = best.min(allocations() - before);
        std::hint::black_box(probe);
    }
    let probe_key = ObjectKey::new("another-key.bin").unwrap();
    assert_eq!(
        best, 1,
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
    //
    // The counter is process-global: the harness (and formerly the sibling
    // sanity test) can allocate on other threads mid-loop. Per-op deltas
    // with a MEDIAN assertion keep the gate exact on the true cost (a real
    // +1/op regression shifts the median) while rare background noise —
    // which can only ADD counts — cannot fail it.
    let mut deltas = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let before = allocations();
        let data = block_once(store.get(&key)).unwrap();
        assert_eq!(&data[..], PAYLOAD);
        // Refcount bump on the payload: no allocation.
        let _handle = data.clone();
        deltas.push(allocations() - before);
    }
    deltas.sort_unstable();
    assert_eq!(
        deltas[ITERATIONS / 2],
        1,
        "warm-key get median must be exactly 1/op (the async_trait future \
         box): deltas={deltas:?}"
    );

    // --- Warm-key overwrite put: small, stable, bounded count. ---
    // Budget ≤ 5/op: 1 (the async_trait future box) + 1 (caller's
    // key.clone() at the call site) + 1 (metadata key clone) + 1
    // (guessed content-type string) + 1 (sha256 hex digest). The
    // constructor takes the content type by value and the call site moves
    // it (no throwaway default-string alloc, no redundant clone).
    let mut deltas = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let before = allocations();
        block_once(store.put(key.clone(), Bytes::from_static(PAYLOAD))).unwrap();
        deltas.push(allocations() - before);
    }
    deltas.sort_unstable();
    assert!(
        deltas[ITERATIONS / 2] <= 5,
        "overwrite put median must be ≤ 5/op: deltas={deltas:?} — a regression slipped in"
    );

    // --- Delivered bytes: caller dropping the payload does not count; a
    // cloned key passed to put is the caller's allocation (1 per op:
    // key.clone() below), asserting the *crate-side* overhead stays low.
    // Best-of-5: a single sample is exposed to rare background allocations,
    // so retry rather than flake.
    let cloned = key.clone(); // caller-side key copy (1 alloc, not ours)
    let mut best = usize::MAX;
    for _ in 0..5 {
        let before = allocations();
        block_once(store.put(cloned.clone(), Bytes::from_static(PAYLOAD))).unwrap();
        best = best.min(allocations() - before);
    }
    assert!(
        best <= 5,
        "crate-side put overhead ≤ 5 allocations/op (caller key clone + \
         box + metadata); best of 5 = {best}"
    );

    std::hint::black_box(probe_key);
}

// Sanity guard: if this fails, the assertions above prove nothing (the
// counter would be broken, not the path miraculously free).
#[test]
fn allocation_counter_sanity() {
    let _guard = SERIAL.lock().unwrap();
    let before = allocations();
    std::hint::black_box(format!("fresh-{before}"));
    assert!(
        allocations() > before,
        "format! must allocate (counter sanity check)"
    );
}
