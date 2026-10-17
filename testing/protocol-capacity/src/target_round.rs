//! Explicit synthetic bench work. Never attach these identities to a radio.
use alloc::{vec, vec::Vec};

/// One bounded round, with alternating live blocks to exercise fragmentation.
/// The caller supplies the board allocator's occupied-byte measurement.
#[inline(never)]
pub fn run(used: fn() -> usize, refuse_oversize: fn() -> bool, checkpoint: fn()) -> [usize; 4] {
    let initial = used();
    let mut holes: Vec<Option<Vec<u8>>> = (0..32).map(|_| Some(vec![0x5a; 257])).collect();
    for slot in holes.iter_mut().step_by(2) {
        *slot = None;
    }
    let fragmented = used();
    let residents = crate::current::workload_with_checkpoint(checkpoint);
    core::hint::black_box(&residents);
    let retained = used();
    drop(residents);
    assert_eq!(used(), fragmented, "workload did not release allocations");
    // ARM's worker runs queue pressure after this frame returns. The protocol
    // fixture has already dropped its state; stacking both large temporary
    // constructor frames would measure artificial nesting rather than coexisting
    // retained state. Other targets keep the stronger nested campaign.
    #[cfg(not(target_arch = "arm"))]
    crate::runtime_pressure::run(checkpoint);
    assert_eq!(
        used(),
        fragmented,
        "runtime failure/restart leaked allocations"
    );
    // Use a direct allocator operation, not the global allocation shim: LLVM
    // may elide an unused alloc/free pair even when its null result is inspected.
    assert!(refuse_oversize(), "oversize allocation was not refused");
    assert_eq!(used(), fragmented, "refusal changed allocator occupancy");
    drop(holes);
    let final_used = used();
    assert_eq!(final_used, initial, "fragmentation guards leaked");
    [initial, fragmented, retained, final_used]
}
