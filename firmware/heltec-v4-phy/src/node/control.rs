//! Signed commands and status in the node loop (D1: "Commands and status").
//!
//! The same carrier, runtime and quiet window the modem loop uses. Control frames are told
//! apart from node host lines the way the modem loop tells them from direct-PHY bytes: a KISS
//! FEND at the node's line boundary opens a frame, and every byte outside a frame goes to the
//! node's line parser in order. Replies go through the loop's `BoundedHost`.

use embassy_time::Duration;
use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::channel::{Channel as _, ChannelInfo as _, Event};
use radio_hand::link::HostLink;

use super::V4NodeChannel;
use crate::channels::{self, ControlDemux, ControlFrameStream};
use crate::control_boot::ControlReady;
use crate::control_carrier::{self, ControlCarrier};
use crate::radio_owner::V4RadioOwner;

/// How long the loop waits before retrying a rollback the radio was not quiet for. The
/// modem loop's figure.
const EXPIRY_RETRY: Duration = Duration::from_millis(200);

pub struct NodeControl {
    ready: Option<ControlReady>,
    stream: ControlFrameStream,
    carrier: ControlCarrier,
}

impl NodeControl {
    /// `ready` is the control runtime boot recovered; `None` on an unclaimed board, which
    /// answers no control frame, as the modem loop does.
    pub const fn new(ready: Option<ControlReady>) -> Self {
        Self {
            ready,
            stream: ControlFrameStream::new(),
            carrier: ControlCarrier::new(),
        }
    }

    /// The armed candidate's rollback deadline, in board milliseconds.
    pub fn deadline_ms(&self) -> Option<u64> {
        self.ready
            .as_ref()
            .and_then(|ready| ready.runtime.provisional_deadline_ms())
    }

    /// Whether these host bytes belong to the control stream rather than the probes.
    pub fn claims(&self, packet: &[u8]) -> bool {
        self.stream.in_frame() || packet.contains(&selvage::kiss::FEND)
    }

    /// The deadline fired: roll back, or wait a moment when the radio is not quiet yet.
    pub async fn expire<RK: RadioKind, DLY: DelayNs>(&mut self, owner: &mut V4RadioOwner<RK, DLY>) {
        if let Some(ready) = self.ready.as_mut()
            && !control_carrier::expire_provisional(owner, ready).await
        {
            embassy_time::Timer::after(EXPIRY_RETRY).await;
        }
    }

    /// Demultiplex one host read byte by byte: node line bytes to the channel, a completed
    /// status request answered from the boot snapshot, a completed command to the carrier.
    pub async fn host_bytes<RK, DLY, L>(
        &mut self,
        owner: &mut V4RadioOwner<RK, DLY>,
        channel: &mut V4NodeChannel,
        host: &mut L,
        packet: &[u8],
    ) where
        RK: RadioKind,
        DLY: DelayNs,
        L: HostLink,
    {
        for &byte in packet {
            match self.stream.demux_byte(channel.at_boundary(), byte) {
                ControlDemux::Ordinary => {
                    let mut exec = owner.executive();
                    let _ = channel
                        .serve(&mut exec, host, Event::HostBytes(&[byte]))
                        .await;
                }
                ControlDemux::Consumed => {}
                ControlDemux::StatusRequest(request) => {
                    if let Some(ready) = self.ready.as_ref() {
                        channels::send_control_status(
                            ready.snapshot.with_query_nonce(request.nonce()),
                            host,
                        )
                        .await;
                    }
                }
                ControlDemux::Command => {
                    if let Some(ready) = self.ready.as_mut() {
                        self.carrier
                            .serve(self.stream.frame(), owner, ready, host)
                            .await;
                    }
                }
            }
        }
    }
}
