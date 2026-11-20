//! The board's heap.
//!
//! Until N3 this firmware had no allocator at all, and the N0 receipt could record a heap
//! high-water mark of zero by construction. Linking `retinue` ends that: its sans-io core is
//! `no_std + alloc`, so packet payloads and reassembly buffers are heap-allocated. The
//! heltec doc's done condition asks for a heap high-water figure precisely because this
//! moment was expected.
//!
//! The allocator type, and the guarantees that replace "no heap", are shared with every board
//! that runs the node channel: see [`radio_hand::heap`]. This file keeps what is the T114's own:
//! the global allocator static, its fixed memory, and its size.

use radio_hand::heap::TrackingHeap;

#[global_allocator]
static HEAP: TrackingHeap = TrackingHeap::empty();

/// Bytes reserved for the heap.
///
/// Sized against the bounded tables above it: one resource reassembly at
/// `MAX_RESOURCE_PARTS` parts is roughly 14 KB, link and packet buffers are a few KB more,
/// and the rest is headroom so fragmentation has somewhere to go. The linker accounts
/// for this reservation together with all other static RAM; runtime acceptance still
/// requires measuring the selected workload and its stack separately.
pub const HEAP_SIZE: usize = 48 * 1024;

static mut HEAP_MEMORY: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Hand the heap its memory. Call once, before anything allocates.
///
/// # Safety
///
/// Must be called exactly once, and before any allocation. Both hold because the only
/// caller is the first statement of `main`.
pub unsafe fn init(spawner: embassy_executor::Spawner) {
    let start = &raw const HEAP_MEMORY as usize;
    unsafe { HEAP.init(start, HEAP_SIZE) };
    let _ = spawner;
}

/// Bytes currently allocated. The figure a receipt reports.
pub fn used() -> usize {
    HEAP.used()
}

/// Bytes still available.
pub fn free() -> usize {
    HEAP.free()
}

/// Largest live allocation observed since boot.
///
/// Unlike [`used`], this survives packet-buffer release and is the figure a sustained flood
/// receipt needs. It resets only on reboot, alongside every other board counter.
pub fn high_water() -> usize {
    HEAP.high_water()
}

/// Failed allocation calls since boot, including a failed realloc's new allocation.
pub fn allocation_failures() -> usize {
    HEAP.allocation_failures()
}
