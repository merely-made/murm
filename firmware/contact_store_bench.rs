//! Before-radio storage campaign. This fixture's identities never reach RF.
use core::{
    fmt::Write,
    sync::atomic::{AtomicU32, Ordering},
};
use radio_hand::tucket_contacts::{self, Storage};
static RESULT: AtomicU32 = AtomicU32::new(u32::MAX);
#[allow(dead_code)] // Used only by the explicit T114 fixture-handoff image.
pub fn handoff(store: &mut impl Storage) {
    let value = match tucket_contacts::fixture::clear_fixture(store) {
        Ok(()) => 0,
        Err(fault) => 0xf0000000 | fault as u32,
    };
    RESULT.store(value, Ordering::Relaxed);
}
pub fn run(store: &mut impl Storage) {
    let value = match tucket_contacts::fixture::run(store) {
        Ok(r) if r.sequence < 65536 => {
            ((r.initial_keys as u32) << 24)
                | ((r.final_keys as u32) << 20)
                | ((r.commits as u32) << 16)
                | r.sequence
        }
        Ok(_) => 0xf0000000,
        Err(fault) => 0xf0000000 | (fault as u32),
    };
    RESULT.store(value, Ordering::Relaxed);
}
pub fn report() -> radio_face::Text<192> {
    let value = RESULT.load(Ordering::Relaxed);
    let mut reply = radio_face::Text::empty();
    if value >> 28 == 0xf {
        let _ = write!(reply, "contacts bench failed code={}\r\n", value & 0xffff);
    } else {
        let _ = write!(
            reply,
            "contacts bench ok initial_keys={} keys={} commits={} sequence={}\r\n",
            value >> 24,
            (value >> 20) & 15,
            (value >> 16) & 15,
            value & 0xffff
        );
    }
    reply
}
