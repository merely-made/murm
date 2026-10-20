//! Opt-in, volatile USB-commissioned Tucket-only service. No other protocol
//! identity or nonce lease is instantiated. Settings and custody stay unchanged.
extern crate alloc;
use alloc::boxed::Box;
use core::fmt::Write;
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use lora_phy::{DelayNs, mod_traits::RadioKind};
use radio_hand::{
    executive::Executive,
    link::HostLink,
    resident_command::{self, Command},
    resident_wire::ResidentSetup,
    tucket_contacts::{Scope, Snapshot},
    tucket_endpoint::{Endpoint, Report},
};
use tucket::{
    identity::LocalIdentity,
    instance::{Instance, InstanceConfig, SendTiming},
    node::{Node, NodeCapacity, TextRetryPolicy},
};

pub struct State {
    endpoint: Endpoint,
    cache: Snapshot,
}
struct Prepared {
    setup: ResidentSetup,
    cache: Snapshot,
}
static REQUEST: Signal<CriticalSectionRawMutex, Prepared> = Signal::new();
static RESULT: Signal<CriticalSectionRawMutex, Result<Box<State>, ()>> = Signal::new();
#[inline(never)]
fn build(p: Prepared) -> Result<Box<State>, ()> {
    let mut instance = Instance::new(
        Node::with_capacity(
            LocalIdentity::from_seed(p.setup.tucket_identity_seed),
            false,
            NodeCapacity::new(8, 32).map_err(|_| ())?,
        )
        .map_err(|_| ())?,
        InstanceConfig::for_radio(
            2,
            p.setup.tx_budget_ms,
            tucket::instance::DEFAULT_REPLY_DELAY_MS,
        )
        .ok_or(())?,
    );
    instance.restore_contacts(&p.cache.keys).map_err(|_| ())?;
    Ok(Box::new(State {
        endpoint: Endpoint::new(
            Instant::now().as_millis(),
            instance,
            p.setup.tx_budget_ms,
            p.setup.frame_ttl_ms,
        )
        .map_err(|_| ())?,
        cache: p.cache,
    }))
}
#[embassy_executor::task]
async fn builder() {
    RESULT.signal(build(REQUEST.wait().await));
}
pub fn start_builder(spawner: Spawner) {
    spawner.spawn(builder().expect("one Tucket constructor"));
}
pub async fn accept_setup<RK: RadioKind, DLY: DelayNs>(
    stream: &mut radio_hand::resident_wire::ResidentSetupStream,
    boundary: bool,
    packet: &[u8],
    exec: &mut Executive<'_, RK, DLY>,
    host: &mut impl HostLink,
) -> bool {
    use radio_hand::resident_wire::{CMD_RESIDENT_SETUP, ResidentSetupByte};
    if !stream.pending() && !(boundary && packet.first() == Some(&CMD_RESIDENT_SETUP)) {
        return false;
    }
    for (index, &byte) in packet.iter().enumerate() {
        match stream.push(boundary, byte) {
            ResidentSetupByte::Complete(setup) => {
                commission(setup, exec, host, &packet[index + 1..]).await;
                break;
            }
            ResidentSetupByte::Rejected(_) => {
                let _ = host.write_all(b"resident setup refused\r\n").await;
            }
            _ => {}
        }
    }
    true
}

/// Reject unsupported role requests before touching the radio or flash. A valid
/// v1 setup uses just its Tucket profile/seed and common timing fields here.
pub async fn commission<RK: RadioKind, DLY: DelayNs>(
    setup: ResidentSetup,
    exec: &mut Executive<'_, RK, DLY>,
    host: &mut impl HostLink,
    tail: &[u8],
) {
    if setup.home != 2
        || setup.pin.is_some_and(|id| id != 2)
        || setup.require_coverage
        || setup.frame_ttl_ms <= setup.tx_budget_ms
        || InstanceConfig::for_radio(
            2,
            setup.tx_budget_ms,
            tucket::instance::DEFAULT_REPLY_DELAY_MS,
        )
        .is_none()
    {
        let _ = host
            .write_all(b"resident setup refused: Tucket only\r\n")
            .await;
        return;
    }
    let public = LocalIdentity::from_seed(setup.tucket_identity_seed)
        .identity()
        .pub_key;
    let scope = Scope::new(public, setup.profiles[2]);
    // Drain the old personality's completed frame before changing its profile.
    let mut discarded = [0; 255];
    exec.collect(&mut discarded)
        .await
        .unwrap_or_else(|_| reset());
    let cache = exec.tucket_contacts(scope, None).await;
    let cache = match cache {
        Ok(Some(cache)) => cache,
        Ok(None) => Snapshot::new(scope, &[]).unwrap_or_else(|_| reset()),
        Err(cause) => {
            let mut line = radio_face::Text::<96>::empty();
            let _ = write!(line, "resident setup refused: contacts {cause:?}\r\n");
            let _ = host.write_all(line.as_str().as_bytes()).await;
            return;
        }
    };
    if exec.apply_profile(&setup.profiles[2]).await != selvage::CONFIG_ACCEPTED {
        reset();
    }
    exec.ensure_rx().await.unwrap_or_else(|_| reset());
    REQUEST.signal(Prepared { setup, cache });
    let mut state = RESULT.wait().await.unwrap_or_else(|_| reset());
    serve(&mut state, exec, host, tail).await
}
fn reset() -> ! {
    cortex_m::peripheral::SCB::sys_reset()
}
// Format before constructing the future: retaining Report in every await slot
// would waste several KiB of the main task's static SRAM reservation.
fn report(host: &mut impl HostLink, out: Report) -> impl Future<Output = ()> + '_ {
    let mut line = Box::new(out.format());
    async move {
        if host.is_attached() {
            let _ = host.write_all(line.as_bytes()).await;
        }
        line.clear();
    }
}
async fn status(state: &State, host: &mut impl HostLink) {
    let mut line = radio_face::Text::<512>::empty();
    let _ = writeln!(
        line,
        "resident status state=Home active=Some(Activation {{ instance: PersonalityId(2) }}) role=TucketEndpoint"
    );
    let _ = writeln!(
        line,
        "resident contacts persisted={} installed={} max=8",
        state.cache.keys.len(),
        state.endpoint.instance().node().contact_keys().count()
    );
    let _ = writeln!(
        line,
        "resident memory heap_capacity={} heap_allocator_used={} heap_allocator_used_peak={} heap_alloc_failures={}",
        crate::heap::HEAP_SIZE,
        crate::heap::used(),
        crate::heap::high_water(),
        crate::heap::allocation_failures()
    );
    #[cfg(feature = "memory-bench")]
    {
        let (free, total) = crate::stack_watermark::measure();
        let _ = writeln!(line, "resident watermark free={free} total={total}");
    }
    let _ = host.write_all(line.as_str().as_bytes()).await;
}
async fn read(
    host: &mut impl HostLink,
    bytes: &mut [u8],
) -> Result<usize, radio_hand::link::LinkFault> {
    if !host.is_attached() {
        host.attached().await;
    }
    host.read(bytes).await
}
async fn receive<RK: RadioKind, DLY: DelayNs>(
    state: &mut State,
    exec: &mut Executive<'_, RK, DLY>,
    host: &mut impl HostLink,
    bytes: &mut [u8],
) {
    if let Some(rx) = exec.collect(bytes).await.unwrap_or_else(|_| reset()) {
        match state
            .endpoint
            .ingest(Instant::now().as_millis(), &bytes[..rx.len])
        {
            Ok(out) => report(host, out).await,
            Err(radio_hand::tucket_endpoint::Error::Clock) => reset(),
            Err(_) => { /* Unauthenticated/invalid RF is not a board fault. */ }
        }
    }
}
async fn save<RK: RadioKind, DLY: DelayNs>(state: &mut State, exec: &mut Executive<'_, RK, DLY>) {
    let mut keys = heapless::Vec::<[u8; 32], 8>::new();
    for key in state.endpoint.instance().node().contact_keys() {
        keys.push(key).unwrap_or_else(|_| reset());
    }
    keys.sort_unstable();
    if keys == state.cache.keys {
        return;
    }
    let snapshot = Snapshot::new(state.cache.scope, &keys).unwrap_or_else(|_| reset());
    exec.tucket_contacts(snapshot.scope, Some(&snapshot))
        .await
        .unwrap_or_else(|_| reset());
    state.cache = snapshot;
}
async fn command(state: &mut State, host: &mut impl HostLink, command: Command) {
    let now = Instant::now().as_millis();
    let result = match command {
        Command::Status => {
            status(state, host).await;
            return;
        }
        Command::TucketText {
            to,
            timestamp,
            ttl_ms,
            attempts,
            flood_last,
            text,
        } => {
            if let (Some(until), Some(policy)) = (
                now.checked_add(ttl_ms),
                TextRetryPolicy::new(attempts, flood_last),
            ) {
                state
                    .endpoint
                    .send(
                        now,
                        to,
                        &text,
                        policy,
                        SendTiming {
                            timestamp,
                            expires_at: until,
                            allowed_until: until,
                        },
                    )
                    .map(|_| Report::default())
            } else {
                Err(radio_hand::tucket_endpoint::Error::Overflow)
            }
        }
        Command::TucketAdvert { timestamp, data } => {
            state.endpoint.advertise(now, timestamp, &data)
        }
        _ => Err(radio_hand::tucket_endpoint::Error::Configuration),
    };
    match result {
        Ok(out) => report(host, out).await,
        Err(_) => {
            let _ = host.write_diagnostic(b"resident refused\r\n").await;
        }
    }
}
async fn serve<RK: RadioKind, DLY: DelayNs>(
    state: &mut State,
    exec: &mut Executive<'_, RK, DLY>,
    host: &mut impl HostLink,
    tail: &[u8],
) -> ! {
    let mut stream = resident_command::Stream::new();
    let mut usb = [0; 64];
    let mut rf = [0; 255];
    let mut initial = Some(tail);
    loop {
        exec.ensure_rx().await.unwrap_or_else(|_| reset());
        let now = Instant::now().as_millis();
        let wake = state
            .endpoint
            .next_deadline()
            .unwrap_or(now + 50)
            .min(now + 50)
            .max(now + 1);
        let event = if let Some(tail) = initial.take().filter(|t| !t.is_empty()) {
            usb[..tail.len()].copy_from_slice(tail);
            Either3::First(Ok(tail.len()))
        } else {
            select3(
                read(host, &mut usb),
                exec.wait_rx_irq(),
                Timer::at(Instant::from_millis(wake)),
            )
            .await
        };
        match event {
            Either3::Second(Ok(())) => receive(state, exec, host, &mut rf).await,
            Either3::Second(Err(_)) => reset(),
            Either3::First(Ok(n)) => {
                let packet = &usb[..n];
                if stream.at_boundary() && (packet == b"bootloader\n" || packet == b"reboot\n") {
                    if packet == b"bootloader\n" {
                        embassy_nrf::pac::POWER
                            .gpregret()
                            .write(|v| v.set_gpregret(0x4e));
                    }
                    reset();
                }
                for &byte in packet {
                    if let resident_command::Event::Complete(c) = stream.push(byte) {
                        report(
                            host,
                            state
                                .endpoint
                                .poll(Instant::now().as_millis())
                                .unwrap_or_else(|_| reset()),
                        )
                        .await;
                        command(state, host, c).await;
                    }
                }
            }
            Either3::First(Err(_)) => {
                stream = resident_command::Stream::new();
            }
            Either3::Third(()) => {}
        }
        receive(state, exec, host, &mut rf).await;
        save(state, exec).await;
        report(
            host,
            state
                .endpoint
                .poll(Instant::now().as_millis())
                .unwrap_or_else(|_| reset()),
        )
        .await;
        receive(state, exec, host, &mut rf).await;
        save(state, exec).await;
        if let Some(tx) = state
            .endpoint
            .begin_tx(Instant::now().as_millis())
            .unwrap_or_else(|_| reset())
        {
            let sent = with_timeout(
                Duration::from_millis(state.endpoint.budget()),
                exec.transmit(&tx.frame),
            )
            .await
            .unwrap_or_else(|_| reset())
                == selvage::TX_ACCEPTED;
            report(
                host,
                state
                    .endpoint
                    .complete_tx(Instant::now().as_millis(), tx.id, sent)
                    .unwrap_or_else(|_| reset()),
            )
            .await;
        }
    }
}
