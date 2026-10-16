// Host simulation of each firmware heap using its actual LLFF allocator.
// Host ABI/stack and synthetic peers remain distinct from target measurements.
use core::alloc::Layout;
use std::alloc::{GlobalAlloc, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct Tracked;
#[repr(C, align(16))]
struct Storage([u8; HEAP_BYTES]);
static mut STORAGE: Storage = Storage([0; HEAP_BYTES]);
static HEAP: embedded_alloc::LlffHeap = embedded_alloc::LlffHeap::empty();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Tracked = Tracked;

// SAFETY: before activation System owns bootstrap allocations; afterward LLFF
// owns this fixed static region. Deallocation dispatches by that disjoint range,
// so runtime bootstrap objects can still be released after activation.
unsafe impl GlobalAlloc for Tracked {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !ACTIVE.load(Relaxed) {
            // SAFETY: forward the valid allocator layout unchanged.
            return unsafe { System.alloc(layout) };
        }
        // SAFETY: initialized once before ACTIVE, with the exclusive live region.
        let ptr = unsafe { HEAP.alloc(layout) };
        if ptr.is_null() { FAILURES.fetch_add(1, Relaxed); }
        PEAK.fetch_max(HEAP.used(), Relaxed);
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let base = core::ptr::addr_of_mut!(STORAGE) as usize;
        if (base..base + HEAP_BYTES).contains(&(ptr as usize)) {
            // SAFETY: the allocator contract supplies a live LLFF allocation and
            // its original layout; System allocations cannot overlap STORAGE.
            unsafe { HEAP.dealloc(ptr, layout) };
        } else {
            // SAFETY: every allocation outside STORAGE came from System.
            unsafe { System.dealloc(ptr, layout) };
        }
    }
}

fn main() {
    // SAFETY: exclusive single initialization before any LLFF allocation. The
    // aligned static buffer outlives all of them and ACTIVE is never reset.
    unsafe { HEAP.init(core::ptr::addr_of_mut!(STORAGE).cast::<u8>() as usize, HEAP_BYTES) };
    ACTIVE.store(true, Relaxed);
    let mut holes: Vec<Option<Vec<u8>>> = (0..32).map(|_| Some(vec![0x5a; 257])).collect();
    for slot in holes.iter_mut().step_by(2) { *slot = None; }
    let baseline = HEAP.used();
    let mut retained = 0;
    for _ in 0..16 {
        let residents = protocol_capacity_probe::current::workload();
        retained = HEAP.used() - baseline;
        core::hint::black_box(&residents);
        drop(residents);
        assert_eq!(HEAP.used(), baseline, "repeat leaked or retained work");
    }
    assert_eq!(FAILURES.load(Relaxed), 0, "unexpected allocation refusal");
    let before_refusal = HEAP.used();
    // SAFETY: deliberately oversized valid layout. A non-null result would be
    // released with the same allocator/layout before asserting the test failure.
    let layout = Layout::from_size_align(HEAP_BYTES * 2, 16).unwrap();
    let denied = unsafe { std::alloc::alloc(layout) };
    if !denied.is_null() { unsafe { std::alloc::dealloc(denied, layout) }; }
    assert!(denied.is_null());
    assert_eq!(HEAP.used(), before_refusal);
    assert_eq!(FAILURES.load(Relaxed), 1);
    drop(holes);
    assert_eq!(HEAP.used(), 0);
    let peak = PEAK.load(Relaxed);
    println!("{{\"scope\":\"host_LLFF_simulation_not_target_runtime\",\"heap_capacity\":{HEAP_BYTES},\"rounds\":16,\"fragmented_baseline_occupied\":{baseline},\"retained_workload_occupied\":{retained},\"occupied_peak\":{peak},\"expected_allocation_refusals\":1,\"unexpected_allocation_refusals\":0,\"after_drop_occupied\":0}}");
}
