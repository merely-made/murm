//! A fixed-region heap that remembers its peak, for boards that link the node channel.
//!
//! The node channel is the one part of this crate that allocates, because `retinue`'s sans-io
//! core is `no_std + alloc`. Each board that runs it needs the same measured allocator, so the
//! type lives here once; each firmware keeps its own `#[global_allocator]` static, its own fixed
//! memory, and its own size.
//!
//! What replaces "no heap" as the guarantee:
//!
//! - **The heap is a fixed array** owned by the firmware, sized there and never grown. It cannot
//!   take memory from anything else, and the linker accounts for it as ordinary static storage.
//! - **Every table above it is bounded** by the node's capacity parameters, so the number of
//!   live allocations has a ceiling rather than depending on what a peer sends.
//! - **Usage is measurable** through [`TrackingHeap::used`], [`TrackingHeap::free`] and
//!   [`TrackingHeap::high_water`], so a receipt can state the real peak rather than mistaking a
//!   post-flood live allocation for one.
//!
//! A linked-list-first-fit allocator is chosen over TLSF because this workload is a small
//! number of short-lived buffers rather than a churn of many sizes, and LLFF is the simpler
//! thing to reason about when it goes wrong.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use embedded_alloc::LlffHeap;

/// The firmware cannot reconstruct a peak from a current allocation after a packet buffer
/// has been released. Keep the peak beside the fixed allocator instead, updating it only
/// after a successful allocation. This is a measurement aid, not a second allocator.
pub struct TrackingHeap {
    inner: LlffHeap,
    high_water: AtomicUsize,
    allocation_failures: AtomicUsize,
}

impl TrackingHeap {
    /// An allocator with no memory yet. Every allocation fails until [`Self::init`].
    pub const fn empty() -> Self {
        Self {
            inner: LlffHeap::empty(),
            high_water: AtomicUsize::new(0),
            allocation_failures: AtomicUsize::new(0),
        }
    }

    /// Hand the allocator its fixed region.
    ///
    /// # Safety
    ///
    /// Call exactly once, before any allocation, with a region of `size` bytes at
    /// `start_addr` that nothing else uses for the life of the program.
    #[allow(unsafe_code)]
    pub unsafe fn init(&self, start_addr: usize, size: usize) {
        // SAFETY: forwarded from this function's one-shot contract.
        unsafe { self.inner.init(start_addr, size) };
    }

    fn note_high_water(&self) {
        let used = self.inner.used();
        let mut known = self.high_water.load(Ordering::Relaxed);
        while used > known {
            match self.high_water.compare_exchange_weak(
                known,
                used,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => known = current,
            }
        }
    }

    /// Bytes currently allocated. The figure a receipt reports.
    pub fn used(&self) -> usize {
        self.inner.used()
    }

    /// Bytes still available.
    pub fn free(&self) -> usize {
        self.inner.free()
    }

    /// Largest live allocation observed since boot.
    ///
    /// Unlike [`Self::used`], this survives packet-buffer release and is the figure a sustained
    /// flood receipt needs. It resets only on reboot, alongside every other board counter.
    pub fn high_water(&self) -> usize {
        self.high_water.load(Ordering::Relaxed)
    }

    /// Failed allocation calls since boot, including a failed realloc's new allocation.
    pub fn allocation_failures(&self) -> usize {
        self.allocation_failures.load(Ordering::Relaxed)
    }
}

#[allow(unsafe_code)]
unsafe impl GlobalAlloc for TrackingHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `GlobalAlloc` receives a valid layout from the allocation runtime; the
        // wrapped allocator owns the fixed region initialised before any allocation.
        let allocation = unsafe { self.inner.alloc(layout) };
        if !allocation.is_null() {
            self.note_high_water();
        } else {
            self.allocation_failures.fetch_add(1, Ordering::Relaxed);
        }
        allocation
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: allocations passed back by the runtime came from `alloc` above.
        unsafe { self.inner.dealloc(ptr, layout) };
    }
}
