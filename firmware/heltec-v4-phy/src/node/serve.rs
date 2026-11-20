//! The node loop, forever.
//!
//! Modelled on `channels::serve_rnode`: the owner keeps exclusive custody of the radio and
//! flash, only the interrupt wait is raced, and collection is never raced. Beside it run the
//! node's own heartbeat and `wake_at` deadline, as on the T114, so announces, relays and link
//! timers keep time with nobody attached.
//!
//! No banner and no session: USB startup stays quiet, and the host only observes. Every host
//! write goes through [`BoundedHost`], so nothing a host does or fails to do holds the radio.

use embassy_futures::select::{Either3, select3};
use lora_phy::DelayNs;
use lora_phy::mod_traits::RadioKind;
use radio_hand::channel::{Channel as _, ChannelInfo as _, Event};
use radio_hand::executive::Heartbeat;
use radio_hand::link::HostLink;

use super::V4NodeChannel;
use super::bounded_host::BoundedHost;
use crate::control_boot::ControlReady;
use crate::radio_owner::{RxSetupFault, V4RadioOwner};
use crate::{channels, heap, power, wake_input};

/// The board as a node, forever.
///
/// `control_ready` is held for the life of the loop: the next checkpoint serves signed
/// commands from it here, through the owner's quiet window, at host line boundaries.
pub async fn serve<RK, DLY, L>(
    mut owner: V4RadioOwner<RK, DLY>,
    channel: &'static mut V4NodeChannel,
    _control_ready: Option<ControlReady>,
    online: &[u8],
    identity_line: &[u8],
    host: L,
) -> !
where
    RK: RadioKind,
    DLY: DelayNs,
    L: HostLink,
{
    let mut host = BoundedHost::new(host);
    // Outside the loop, so a busy host cannot reset the announce cadence.
    let mut heartbeat = Heartbeat::new(channel.heartbeat());

    loop {
        {
            let _awake = power::Awake::new();
            match owner.ensure_rx().await {
                Ok(true) => owner.radio_online(),
                Ok(false) => {}
                Err(RxSetupFault::Prepare) => {
                    owner.radio_fault(5, "RX SETUP");
                    embassy_time::Timer::after_millis(250).await;
                    continue;
                }
                Err(RxSetupFault::Arm) => {
                    owner.radio_fault(5, "RX ARM");
                    embassy_time::Timer::after_millis(250).await;
                    continue;
                }
            }
        }

        let mut host_packet = [0_u8; 64];
        let mut radio_frame = [0_u8; selvage::MAX_RADIO_FRAME_LEN];
        // Only the interrupt wait is raced. A completed level-high IRQ wins at once, so a
        // deadline already due cannot repeatedly cancel the waiter before collection.
        let radio_wait = async {
            if wake_input::radio_is_high() {
                Ok(())
            } else {
                owner.wait_rx_irq().await
            }
        };
        let woken = select3(
            host.read(&mut host_packet),
            radio_wait,
            heartbeat.next(channel.wake_at()),
        )
        .await;
        let _awake = power::Awake::new();
        match woken {
            Either3::Second(Ok(())) => {
                // Deliberately not raced: the frame is in the radio until it is read out.
                let Some(frame) = (match owner.collect(&mut radio_frame).await {
                    Ok(frame) => frame,
                    Err(_) => {
                        owner.radio_fault(6, "RADIO RX");
                        continue;
                    }
                }) else {
                    continue;
                };
                // The channel counts and shows the frame itself once it decodes.
                let mut exec = owner.executive();
                let _ = channel
                    .serve(
                        &mut exec,
                        &mut host,
                        Event::RadioFrame {
                            frame: &radio_frame[..frame.len],
                            rssi: frame.rssi,
                            snr: frame.snr,
                        },
                    )
                    .await;
            }
            Either3::Second(Err(_)) => owner.radio_fault(6, "RADIO RX"),
            Either3::Third(tick) => {
                let mut exec = owner.executive();
                let _ = channel.serve(&mut exec, &mut host, tick).await;
            }
            Either3::First(Err(_)) | Either3::First(Ok(0)) => {}
            Either3::First(Ok(length)) => {
                owner.note_host_activity();
                let packet = &host_packet[..length];
                if channel.at_boundary() {
                    if packet == b"heap\n" || packet == b"heap\r\n" {
                        let _ = host.write_all(heap::report().as_str().as_bytes()).await;
                        continue;
                    }
                    if matches!(
                        owner.probe(packet, online, identity_line, &mut host).await,
                        channels::Outcome::Served
                    ) {
                        continue;
                    }
                }
                let mut exec = owner.executive();
                let _ = channel
                    .serve(&mut exec, &mut host, Event::HostBytes(packet))
                    .await;
            }
        }
    }
}
