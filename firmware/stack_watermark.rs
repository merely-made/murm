//! Bench-only boot-painted stack evidence. Never repaint a running stack.
//!
//! ARM uses cortex-m-rt's reset-time paint-stack feature. Xtensa's bench entry
//! paints below its live SP before constructing the executor or application.
//! Unpainted startup frames count as used, making that upper bound conservative.

#[cfg(target_arch = "arm")]
const PAINT: u32 = 0xcccc_cccc;
#[cfg(target_arch = "xtensa")]
const PAINT: u32 = 0xa5c3_6d9e;

fn bounds() -> (usize, usize) {
    unsafe extern "C" {
        #[cfg(target_arch = "arm")]
        static __sheap: u32;
        #[cfg(target_arch = "arm")]
        static _stack_start: u32;
        #[cfg(target_arch = "xtensa")]
        static _stack_end_cpu0: u32;
        #[cfg(target_arch = "xtensa")]
        static _stack_start_cpu0: u32;
    }
    #[cfg(target_arch = "arm")]
    return (
        core::ptr::addr_of!(__sheap) as usize,
        core::ptr::addr_of!(_stack_start) as usize,
    );
    #[cfg(target_arch = "xtensa")]
    return (
        // Leave the RTOS/HAL's bottom stack guard outside the paint scan.
        core::ptr::addr_of!(_stack_end_cpu0) as usize + 64,
        core::ptr::addr_of!(_stack_start_cpu0) as usize,
    );
}

#[cfg(target_arch = "xtensa")]
pub fn paint_before_executor() {
    let sp: usize;
    // SAFETY: read-only SP query, no memory or machine-state changes.
    unsafe { core::arch::asm!("mov {0}, a1", out(reg) sp, options(nomem, nostack)) };
    let (bottom, top) = bounds();
    let end = sp.saturating_sub(256) & !3;
    assert!(bottom <= end && end < top);
    for address in (bottom..end).step_by(4) {
        // SAFETY: linker-owned unused stack, strictly below live startup frames
        // and spill area. No task, IRQ or second core can own these bytes yet.
        unsafe { (address as *mut u32).write_volatile(PAINT) };
    }
}

/// Contiguous untouched bytes above the linker floor and total stack region.
/// Includes all writes since boot painting, not just explicit SP checkpoints.
/// The result is workload evidence, not a proof about unexecuted paths.
pub fn measure() -> (usize, usize) {
    #[cfg(target_arch = "arm")]
    return cortex_m::interrupt::free(|_| scan());
    #[cfg(target_arch = "xtensa")]
    return critical_section::with(|_| scan());
}

fn scan() -> (usize, usize) {
    let (bottom, top) = bounds();
    let mut end = bottom;
    // Single executor core, with interrupts masked for this bounded scan. No
    // second core or independent thread writes this stack during measurement.
    while end < top {
        // SAFETY: reset painted this linker-owned stack as initialized u32
        // words. No reference to stack objects is formed, no writer can race
        // this scan, and the first changed word terminates it.
        if unsafe { (end as *const u32).read_volatile() } != PAINT {
            break;
        }
        end += 4;
    }
    (end - bottom, top - bottom)
}
