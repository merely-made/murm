//! The signed control carrier a board loop calls: one complete command frame in, the runtime's
//! journaled answer out, and the provisional deadline's rollback.
//!
//! Every frame is a complete `retinue::command` envelope. The verifier restored from the durable
//! owner grants checks target, allowlist, counter window and signature; only then does the
//! request reach the [`ControlRuntime`], which journals the accepted counter inside the board's
//! [`QuietWindow`] before any response exists. Refusals are silent by design: an unverified
//! frame earns no reply, and a board that cannot quiet its radio right now never advances the
//! verifier, so an operator's counter is not desynchronized.
//!
//! The board decides when a quiet window may open ([`ControlBoard::quiet_ready`]); the guard it
//! returns holds the board exclusively until it finishes or resets, so no other radio work can
//! interleave with journaling or apply. Both the V4's modem loop and its node loop call this.

use super::{
    COMMIT_TOKEN_LEN, ControlRuntime, ControlVerifier, DurableScratch, FirstWriteStatus,
    MAX_CONTROL_RESPONSE_FRAME_LEN, QuietWindow, Response, RuntimeError, decode_command_frame,
    decode_verified_command, encode_response_frame, restore_control_verifier,
};
use crate::link::HostLink;

/// Largest KISS-escaped response the carrier writes.
const MAX_RESPONSE_WIRE: usize = 2 + MAX_CONTROL_RESPONSE_FRAME_LEN * 2;

/// What a board lends the carrier beyond its quiet window.
pub trait ControlBoard: QuietWindow {
    /// Whether a quiet window may be entered at this completed-event boundary.
    fn quiet_ready(&self) -> bool;
    /// Commit-token bytes from the board's true entropy source.
    fn commit_entropy(&mut self, out: &mut [u8]) -> Result<(), NoEntropy>;
}

/// The board's true entropy source could not fill a commit token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoEntropy;

/// The resident verifier and the rule for when it must be rebuilt.
pub struct ControlCarrier {
    /// `None` means the in-memory verifier may be ahead of the journal and must be rebuilt
    /// from durable grants and counters before it accepts another envelope.
    verifier: Option<ControlVerifier>,
}

impl Default for ControlCarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlCarrier {
    pub const fn new() -> Self {
        Self { verifier: None }
    }

    /// Serve one complete, tagged command frame. Silence on every refusal.
    ///
    /// `now` is read only after verification, when the runtime needs board time for a
    /// provisional deadline. Board milliseconds since boot, never wall time.
    #[allow(clippy::too_many_arguments)]
    pub async fn serve<B, L>(
        &mut self,
        frame: &[u8],
        board: &mut B,
        runtime: &mut ControlRuntime,
        scratch: &mut DurableScratch<'_>,
        first_write: FirstWriteStatus,
        now: impl Fn() -> u64,
        host: &mut L,
    ) where
        B: ControlBoard,
        L: HostLink,
    {
        let Ok(command) = decode_command_frame(frame) else {
            return;
        };
        if runtime.is_poisoned() || runtime.reset_pending() {
            return;
        }
        if !board.quiet_ready() {
            return;
        }
        let verifier = match self.verifier.as_mut() {
            Some(verifier) => verifier,
            None => {
                let Some(restored) = runtime
                    .state()
                    .and_then(|state| restore_control_verifier(state).ok())
                else {
                    return;
                };
                self.verifier.insert(restored)
            }
        };
        // The token is minted before verification so an entropy fault costs nothing: the
        // verifier has not advanced and the frame simply earns silence.
        let mut commit_token = [0_u8; COMMIT_TOKEN_LEN];
        if board.commit_entropy(&mut commit_token).is_err() {
            return;
        }
        let Ok(verified) = verifier.verify(command) else {
            return;
        };
        let outcome = match decode_verified_command(&verified) {
            Ok(inbound) => runtime
                .serve_inbound(board, scratch, &inbound, now(), first_write, commit_token)
                .await
                .map(|outcome| Some(outcome.into_value())),
            // Verified but not a node-addressed WN0 request: the counter still becomes
            // durable, and the sender learns nothing beyond silence.
            Err(_) => runtime
                .record_verified_command(board, scratch, &verified)
                .await
                .map(|_| None)
                .map_err(RuntimeError::widen_apply),
        };
        match outcome {
            Ok(Some(response)) => send_response(&response, host).await,
            Ok(None) => {}
            // The verifier advanced but the journal may not have. Discard it; the next frame
            // rebuilds from whatever the durable state actually says.
            Err(_) => self.verifier = None,
        }
    }
}

/// Roll back an armed candidate whose deadline has passed.
///
/// Returns `false` only when the board is not at a quiet boundary yet; the loop retries
/// shortly. A poisoned or reset-pending runtime counts as done: it serves nothing more.
pub async fn expire_provisional<B: ControlBoard>(
    board: &mut B,
    runtime: &mut ControlRuntime,
    scratch: &mut DurableScratch<'_>,
    now: impl Fn() -> u64,
) -> bool {
    if runtime.is_poisoned() || runtime.reset_pending() {
        return true;
    }
    if !board.quiet_ready() {
        return false;
    }
    runtime.expire(board, scratch, now()).await.is_ok()
}

/// Writes one tagged response. `HostLink::write_all` flushes the transport itself.
async fn send_response<L: HostLink>(response: &Response, host: &mut L) {
    let mut frame = [0_u8; MAX_CONTROL_RESPONSE_FRAME_LEN];
    let mut wire = [0_u8; MAX_RESPONSE_WIRE];
    if let Ok(len) = encode_response_frame(response, &mut frame)
        && let Some(wire_len) = selvage::kiss::encode_into(&frame[..len], &mut wire)
    {
        let _ = host.write_all(&wire[..wire_len]).await;
    }
}
