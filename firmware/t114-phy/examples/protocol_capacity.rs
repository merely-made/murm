//! Build-only capacity image. Synthetic keys, no radio or USB stack.
//! Its placeholders are reservations, not measured board-runtime memory.
#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;
use cortex_m_rt::entry;
use embedded_alloc::LlffHeap;
use protocol_capacity_probe::current::Residents;
use static_cell::StaticCell;

const HEAP_BYTES: usize = 48 * 1024;
#[global_allocator]
static HEAP: LlffHeap = LlffHeap::empty();
static mut HEAP_MEMORY: [u8; HEAP_BYTES] = [0; HEAP_BYTES];
static RESIDENTS: StaticCell<Residents> = StaticCell::new();
static mut RADIO_RUNTIME_RESERVE: [u8; 65_536] = [0; 65_536];
static mut QUEUE_STATE_RESERVE: [u8; 16_384] = [0; 16_384];

#[entry]
fn main() -> ! {
    let _peripherals = embassy_nrf::init(Default::default());
    // SAFETY: one initialization of this exclusively owned static region before
    // any allocation. It remains alive for the entire program.
    unsafe {
        HEAP.init(
            core::ptr::addr_of_mut!(HEAP_MEMORY).cast::<u8>() as usize,
            HEAP_BYTES,
        )
    };
    core::hint::black_box(core::ptr::addr_of_mut!(RADIO_RUNTIME_RESERVE));
    core::hint::black_box(core::ptr::addr_of_mut!(QUEUE_STATE_RESERVE));
    core::hint::black_box(&protocol_capacity_probe::current::PROTOCOL_LAYOUT_V2);
    RESIDENTS.init_with(protocol_capacity_probe::current::workload);
    loop {
        cortex_m::asm::nop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}
