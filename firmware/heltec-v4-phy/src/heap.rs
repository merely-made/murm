//! The node build's heap. The modem image carries no allocator at all.
//!
//! The allocator type and its guarantees are shared with the T114: see [`radio_hand::heap`].
//! This file keeps what is the V4's own: the global allocator static, its fixed memory, its
//! size, and the `heap` probe line, which uses the T114's format so one bench reads both.

use core::fmt::Write as _;

use radio_hand::heap::TrackingHeap;

#[global_allocator]
static HEAP: TrackingHeap = TrackingHeap::empty();

/// Provisional heap bytes; the final size is the owner's choice after a measured run.
pub const HEAP_SIZE: usize = 96 * 1024;

static mut HEAP_MEMORY: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// Hand the heap its memory. Call once, before anything allocates.
///
/// # Safety
///
/// Must be called exactly once, and before any allocation. Both hold because the only
/// caller is the first statement of `main` after the control fixture check, which does not
/// allocate.
pub unsafe fn init() {
    let start = &raw const HEAP_MEMORY as usize;
    unsafe { HEAP.init(start, HEAP_SIZE) };
}

/// The `heap` probe's reply: live and peak allocation, in the T114's format.
pub fn report() -> radio_face::Text<64> {
    let mut reply = radio_face::Text::<64>::empty();
    let _ = write!(
        &mut reply,
        "heap={}/{} highwater={} free={}\r\n",
        HEAP.used(),
        HEAP_SIZE,
        HEAP.high_water(),
        HEAP.free(),
    );
    reply
}
