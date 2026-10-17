//! Resident construction and contact flash retain the cancel-safe quiet owner.
use super::*;
#[cfg(feature = "resident-contact-store")]
use radio_hand::tucket_contacts::{self, Commit, Snapshot};
impl<RK: RadioKind, DLY: DelayNs> V4RadioOwner<RK, DLY> {
    /// Setup/identity material remains volatile. Public contact storage is a
    /// separate opt-in profile beneath this same exclusive flash owner.
    pub(crate) async fn build_resident(
        &mut self,
        now: u64,
        setup: radio_hand::resident_wire::ResidentSetup,
    ) -> Result<crate::resident::ResidentState, crate::resident::BuildError> {
        let mut guard = QuietWindow::enter(self)
            .await
            .map_err(|_| crate::resident::BuildError::Quiet)?;
        let result = crate::resident::build(
            now,
            setup,
            guard.owner.settings.map(|s| s.identity),
            &mut guard.owner.store,
        );
        guard
            .finish()
            .await
            .map_err(|_| crate::resident::BuildError::Quiet)?;
        result
    }
    pub(crate) fn resident_random(
        &mut self,
        out: &mut [u8],
    ) -> Result<(), radio_hand::executive::StoreFault> {
        self.store.fill_true_random(out)
    }
    #[cfg(feature = "resident-contact-store")]
    pub(crate) async fn save_tucket_contacts(&mut self, snapshot: &Snapshot) -> Result<Commit, ()> {
        let mut guard = QuietWindow::enter(self).await.map_err(|_| ())?;
        let result = tucket_contacts::save(&mut guard.owner.store, snapshot).map_err(|_| ());
        guard.finish().await.map_err(|_| ())?;
        result
    }
}
