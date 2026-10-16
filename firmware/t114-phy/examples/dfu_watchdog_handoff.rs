//! Attended application-only DFU bridge for a running inherited watchdog.
//!
//! A software reset leaves WDT running. The old eight-second watchdog can expire
//! during nrfutil's size-based erase wait for a larger application. This tiny
//! image fits a short transfer, waits for that watchdog to reset, then requests
//! serial DFU with WDT stopped. It touches no radio, flash or durable identity.
//! Install only with an already verified application restoration package ready.
#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    if !embassy_nrf::pac::WDT.runstatus().read().runstatus() {
        embassy_nrf::pac::POWER
            .gpregret()
            .write(|value| value.set_gpregret(0x4e));
        cortex_m::peripheral::SCB::sys_reset();
    }
    // Do not pet, sleep or configure the inherited watchdog. Once its reset
    // clears WDT, this same image boots again and takes the branch above.
    loop {
        cortex_m::asm::nop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        cortex_m::asm::nop();
    }
}
