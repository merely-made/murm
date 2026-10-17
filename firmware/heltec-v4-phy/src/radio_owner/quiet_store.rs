//! WN1 journal access retains the borrow-scoped live quiet guard.
use super::*;
impl<RK: RadioKind, DLY: DelayNs> AbSlotStore for V4QuietGuard<'_, RK, DLY> {
    type Error = control_store::ControlError;
    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> Result<(), Self::Error> {
        AbSlotStore::read_slot(&mut self.owner.store, slot, out)
    }
    fn erase_slot(&mut self, slot: Slot) -> Result<(), Self::Error> {
        AbSlotStore::erase_slot(&mut self.owner.store, slot)
    }
    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> Result<(), Self::Error> {
        AbSlotStore::program_slot(&mut self.owner.store, slot, record)
    }
}
