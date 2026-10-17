//! Dedicated public Tucket pair. Only pre-radio/quiet callers may mutate it.
use super::*;
use radio_hand::tucket_contacts::{SLOT_LEN, Storage};
include!(concat!(env!("OUT_DIR"), "/contacts_region.rs"));
const _: () = assert!(CONTACTS_LENGTH as usize == 2 * PAGE_SIZE && SLOT_LEN <= PAGE_SIZE);
fn offset(slot: Slot) -> u32 {
    CONTACTS_ORIGIN + if slot == Slot::A { 0 } else { PAGE_SIZE as u32 }
}
impl Storage for SettingsStore<'_> {
    type Error = ();
    fn read(&mut self, slot: Slot, out: &mut [u8; SLOT_LEN]) -> Result<(), ()> {
        self.nvmc.read(offset(slot), out).map_err(|_| ())
    }
    fn erase(&mut self, slot: Slot) -> Result<(), ()> {
        count_attempt(&FLASH_ERASE_ATTEMPTS);
        self.nvmc
            .erase(offset(slot), offset(slot) + PAGE_SIZE as u32)
            .map_err(|_| ())
    }
    fn write(&mut self, slot: Slot, relative: usize, bytes: &[u8]) -> Result<(), ()> {
        if !relative.is_multiple_of(4)
            || !bytes.len().is_multiple_of(4)
            || relative + bytes.len() > SLOT_LEN
        {
            return Err(());
        }
        count_attempt(&FLASH_WRITE_ATTEMPTS);
        self.nvmc
            .write(offset(slot) + relative as u32, bytes)
            .map_err(|_| ())
    }
}
