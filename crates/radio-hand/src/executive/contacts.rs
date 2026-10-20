//! Exclusive flash blackout. Caller drains completed RX before entry and must
//! reset if this future is cancelled or any radio step is uncertain.
use super::*;
use crate::tucket_contacts::{Fault, Scope, Snapshot};
impl<RK: RadioKind, DLY: DelayNs> Executive<'_, RK, DLY> {
    pub async fn tucket_contacts(
        &mut self,
        scope: Scope,
        save: Option<&Snapshot>,
    ) -> Result<Option<Snapshot>, Fault> {
        self.invalidate_observation(CONTINUITY_SETTINGS);
        self.radio.prepare_rx = true;
        self.lora.enter_standby().await.map_err(|_| Fault::Io)?;
        self.lora
            .settle_irq_for_quiet_work()
            .await
            .map_err(|_| Fault::Io)?;
        let result = if let Some(snapshot) = save {
            if snapshot.scope != scope {
                Err(Fault::Scope)
            } else {
                self.store.save_tucket_contacts(snapshot).map(|()| None)
            }
        } else {
            self.store.load_tucket_contacts(scope)
        };
        self.ensure_rx().await.map_err(|_| Fault::Io)?;
        result
    }
}
