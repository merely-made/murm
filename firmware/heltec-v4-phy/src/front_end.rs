//! The Heltec V4.2 RF front end: a 17 dB pad into a GC1109 PA/LNA module.
//!
//! From Heltec's V4.2 schematic: `VFEM_Ctrl` (GPIO7) enables the TLV75733 that supplies
//! the module, `PA_CSD` (GPIO2, 10 kΩ pull-down) enables it, `PA_CPS` (GPIO46, 10 kΩ
//! pull-down) selects PA over bypass, and `PA_CTX` is the SX1262's DIO2 RF-switch output.
//! From GEO-CHIP's GC1109 data sheet (Rev0.9.2): CSD=1 CTX=0 is receive through the LNA,
//! CSD=1 CTX=1 transmits through the PA with CPS=1 or the bypass with CPS=0, and supply
//! must precede the control lines.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::peripherals::{GPIO2, GPIO7, GPIO46};
use radio_hand::front_end::FrontEnd;

/// The V4.2 transmit chain, capped at +20 dBm at the antenna (owner ruling 14).
pub const CHAIN: FrontEnd = FrontEnd::PadAndPa {
    pad_db: 17,
    pa_gain_db: 30,
    bypass_loss_db: 1,
    cap_dbm: 20,
    pa_input_max_dbm: 5,
    select_pa,
};

/// The three control outputs, held for the life of the image.
struct Lines {
    _supply: Output<'static>,
    _enable: Output<'static>,
    pa_select: Output<'static>,
}

static LINES: Mutex<CriticalSectionRawMutex, RefCell<Option<Lines>>> =
    Mutex::new(RefCell::new(None));

/// Power the module and enable it, PA deselected. Call before the radio is configured.
pub fn power_up(supply: GPIO7<'static>, enable: GPIO2<'static>, pa_select: GPIO46<'static>) {
    let pa_select = Output::new(pa_select, Level::Low, OutputConfig::default());
    let supply = Output::new(supply, Level::High, OutputConfig::default());
    // The data sheet orders supply before the control lines; the regulator settles well
    // inside a millisecond.
    Delay::new().delay_millis(1);
    let enable = Output::new(enable, Level::High, OutputConfig::default());
    LINES.lock(|lines| {
        *lines.borrow_mut() = Some(Lines {
            _supply: supply,
            _enable: enable,
            pa_select,
        });
    });
}

/// Select the PA (`true`) or the bypass for the transmissions that follow.
fn select_pa(pa: bool) {
    LINES.lock(|lines| {
        if let Some(lines) = lines.borrow_mut().as_mut() {
            lines
                .pa_select
                .set_level(if pa { Level::High } else { Level::Low });
        }
    });
}
