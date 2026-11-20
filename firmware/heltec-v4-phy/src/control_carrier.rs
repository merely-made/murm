//! Normal-runtime signed control carrier on the literal USB Serial/JTAG host stream.
//!
//! The carrier itself is `radio_hand::control::carrier`, shared by the modem loop and the node
//! loop and modelled on the desk there. This file lends it the V4's radio owner: the quiet
//! preflight, the true-entropy commit token, and the owner's `QuietWindow`. `Status` is
//! observed; `ProvisionalApply`, `Commit` and `Revert` run the durable configuration
//! lifecycle, with the candidate applied to the radio only after its rollback record is
//! durable and rolled back on commit timeout or reboot. Refusals are silent by design.

use embassy_time::Instant;
use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::control::carrier::{self, ControlBoard, NoEntropy};
use radio_hand::link::HostLink;

use crate::control_boot::ControlReady;
use crate::radio_owner::{V4QuietPreflight, V4RadioOwner};

impl<RK: RadioKind, DLY: DelayNs> ControlBoard for V4RadioOwner<RK, DLY> {
    fn quiet_ready(&self) -> bool {
        self.quiet_preflight() == V4QuietPreflight::Ready
    }

    fn commit_entropy(&mut self, out: &mut [u8]) -> Result<(), NoEntropy> {
        self.fill_true_random(out).map_err(|_| NoEntropy)
    }
}

/// The resident verifier, over the shared carrier.
pub(crate) struct ControlCarrier(carrier::ControlCarrier);

impl ControlCarrier {
    pub(crate) const fn new() -> Self {
        Self(carrier::ControlCarrier::new())
    }

    /// Serve one complete, tagged command frame from the host stream. Silence on refusal.
    pub(crate) async fn serve<RK, DLY, L>(
        &mut self,
        frame: &[u8],
        owner: &mut V4RadioOwner<RK, DLY>,
        ready: &mut ControlReady,
        host: &mut L,
    ) where
        RK: RadioKind,
        DLY: DelayNs,
        L: HostLink,
    {
        self.0
            .serve(
                frame,
                owner,
                &mut ready.runtime,
                &mut ready.scratch,
                ready.first_write,
                board_now_ms,
                host,
            )
            .await
    }
}

/// Board time for provisional deadlines: milliseconds since this boot. Never wall time, so a
/// deadline can never authorize a candidate after a reboot.
pub(crate) fn board_now_ms() -> u64 {
    Instant::now().as_millis()
}

/// Rolls back an armed candidate whose deadline has passed; `false` means retry shortly.
pub(crate) async fn expire_provisional<RK, DLY>(
    owner: &mut V4RadioOwner<RK, DLY>,
    ready: &mut ControlReady,
) -> bool
where
    RK: RadioKind,
    DLY: DelayNs,
{
    carrier::expire_provisional(owner, &mut ready.runtime, &mut ready.scratch, board_now_ms).await
}
