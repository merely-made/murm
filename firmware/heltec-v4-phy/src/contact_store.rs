//! Opt-in public Tucket cache in its own two sectors, after packet-ID leases.
use crate::store::{SECTOR, SettingsStore};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use radio_hand::{
    store::Slot,
    tucket_contacts::{SLOT_LEN, Storage},
};
pub(crate) const ORIGIN: u32 = crate::store::PACKET_RESERVATION_ORIGIN + 2 * SECTOR;
const _: () = assert!(ORIGIN == 0x3fa000);
const _: () = assert!(ORIGIN + 2 * SECTOR <= 0x400000 && SLOT_LEN <= SECTOR as usize);
fn offset(slot: Slot) -> u32 {
    ORIGIN + if slot == Slot::A { 0 } else { SECTOR }
}
impl Storage for SettingsStore {
    type Error = ();
    fn read(&mut self, slot: Slot, out: &mut [u8; SLOT_LEN]) -> Result<(), ()> {
        self.flash.read(offset(slot), out).map_err(|_| ())
    }
    fn erase(&mut self, slot: Slot) -> Result<(), ()> {
        let start = offset(slot);
        self.flash.erase(start, start + SECTOR).map_err(|_| ())
    }
    fn write(&mut self, slot: Slot, relative: usize, bytes: &[u8]) -> Result<(), ()> {
        if !relative.is_multiple_of(4)
            || !bytes.len().is_multiple_of(4)
            || relative + bytes.len() > SLOT_LEN
        {
            return Err(());
        }
        self.flash
            .write(offset(slot) + relative as u32, bytes)
            .map_err(|_| ())
    }
}
