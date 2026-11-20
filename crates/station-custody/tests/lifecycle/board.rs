//! An in-process V4-shaped board: the real `ControlRuntime` and the real shared carrier
//! (`radio_hand::control::carrier`) behind the literal USB framing, with in-memory flash and a
//! radio that records every configuration applied to it. It rolls an armed candidate back at
//! its board deadline as the V4 loop does, with no host frame, and answers the unsigned
//! `applied` probe at a line boundary with what its radio last applied (D20).

use std::sync::Arc;
use std::time::{Duration, Instant};

use heapless::Vec as BoundedVec;
use radio_hand::applied::{AppliedRadio, Relay};
use radio_hand::control::carrier::{ControlBoard, ControlCarrier, NoEntropy, expire_provisional};
use radio_hand::control::{
    AbSlotStore, BoardRecoveryFacts, BootState, COMMIT_TOKEN_LEN, ConfigApplier, ConfigGeneration,
    ControlRuntime, ControllerRole, DurableConfig, DurableScratch, DurableState, FirstWriteStatus,
    MAX_CONTROL_COMMAND_FRAME_LEN, MAX_DURABLE_BODY, ManagementCarrier, ManagementCarrierSet,
    NodeId, OwnerGrant, PairEvidence, PublicConfigurationV1, QuietExit, QuietGuard, QuietWindow,
    RecoveryClause, RecoveryPathFacts, RecoveryPolicy, ReticulumTransportPolicy, SemanticTagKey,
    next_record,
};
use radio_hand::link::{HostLink, LinkFault};
use radio_hand::region::Region;
use radio_hand::store::Slot;
use retinue::identity::Identity;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::Mutex;

pub const NODE: NodeId = NodeId([0x5a; 16]);
pub const PAGE: usize = 4096;
/// Every commit token this board mints is this byte repeated, so tests can search for it.
pub const TOKEN_BYTE: u8 = 0xc3;

pub fn initial_configuration() -> PublicConfigurationV1 {
    PublicConfigurationV1::new(
        Region::Us915,
        selvage::PhyProfile::meshtastic_long_fast(906_875_000),
        ReticulumTransportPolicy::new(false, false, 0).unwrap(),
        ManagementCarrierSet::from_mask(1).unwrap(),
    )
    .unwrap()
}

fn recovery_facts() -> BoardRecoveryFacts {
    BoardRecoveryFacts::new(
        BoundedVec::from_slice(&[RecoveryPathFacts::new(
            ManagementCarrier::Usb,
            true,
            false,
            false,
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap()
}

#[derive(Default)]
pub struct Flash {
    a: std::vec::Vec<u8>,
    b: std::vec::Vec<u8>,
}

impl AbSlotStore for Flash {
    type Error = ();

    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> Result<(), ()> {
        let page = match slot {
            Slot::A => &self.a,
            Slot::B => &self.b,
        };
        out.copy_from_slice(&page[..out.len()]);
        Ok(())
    }

    fn erase_slot(&mut self, slot: Slot) -> Result<(), ()> {
        match slot {
            Slot::A => self.a.fill(0xff),
            Slot::B => self.b.fill(0xff),
        }
        Ok(())
    }

    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> Result<(), ()> {
        let page = match slot {
            Slot::A => &mut self.a,
            Slot::B => &mut self.b,
        };
        page[..record.len()].copy_from_slice(record);
        Ok(())
    }
}

/// The radio: every configuration it was told to run, in order.
#[derive(Default)]
pub struct Radio {
    pub applied: std::vec::Vec<PublicConfigurationV1>,
}

impl ConfigApplier for Radio {
    type Error = ();

    async fn apply(&mut self, configuration: &DurableConfig) -> Result<(), ()> {
        self.applied.push(configuration.public);
        Ok(())
    }
}

pub struct Hardware {
    pub flash: Flash,
    pub radio: Radio,
}

pub struct Guard<'a> {
    hardware: &'a mut Hardware,
}

impl QuietWindow for Hardware {
    type Error = ();
    type StoreError = ();
    type ApplyError = ();
    type Guard<'a>
        = Guard<'a>
    where
        Self: 'a;

    async fn enter(&mut self) -> Result<Guard<'_>, ()> {
        Ok(Guard { hardware: self })
    }
}

impl QuietGuard for Guard<'_> {
    type Error = ();

    fn abort(&mut self) {}

    async fn finish(&mut self) -> Result<QuietExit, ()> {
        Ok(QuietExit::Resumed)
    }
}

impl AbSlotStore for Guard<'_> {
    type Error = ();

    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> Result<(), ()> {
        self.hardware.flash.read_slot(slot, out)
    }

    fn erase_slot(&mut self, slot: Slot) -> Result<(), ()> {
        self.hardware.flash.erase_slot(slot)
    }

    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> Result<(), ()> {
        self.hardware.flash.program_slot(slot, record)
    }
}

impl ConfigApplier for Guard<'_> {
    type Error = ();

    async fn apply(&mut self, configuration: &DurableConfig) -> Result<(), ()> {
        self.hardware.radio.apply(configuration).await
    }
}

impl ControlBoard for Hardware {
    fn quiet_ready(&self) -> bool {
        true
    }

    fn commit_entropy(&mut self, out: &mut [u8]) -> Result<(), NoEntropy> {
        out.fill(TOKEN_BYTE);
        Ok(())
    }
}

/// What the host link carries back: the carrier's response bytes.
#[derive(Default)]
struct Sink(std::vec::Vec<u8>);

impl HostLink for Sink {
    async fn attached(&mut self) {}

    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, LinkFault> {
        std::future::pending().await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), LinkFault> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

/// The runtime's durable scratch: two slot pages, a body and a page buffer.
type Buffers = ([u8; PAGE], [u8; PAGE], [u8; MAX_DURABLE_BODY], [u8; PAGE]);

pub struct Board {
    pub runtime: ControlRuntime,
    pub hardware: Hardware,
    carrier: ControlCarrier,
    buffers: Box<Buffers>,
    start: Instant,
    served: usize,
    /// Signed frames from another controller, served internally after this many host frames.
    pub interject: Option<(usize, std::vec::Vec<std::vec::Vec<u8>>)>,
    /// How long past its deadline the board waits before rolling back, as a V4 does while its
    /// radio is not quiet. Zero rolls back on time.
    pub expiry_hold: Duration,
}

impl Board {
    /// A claimed board: `owner` holds the owner grant, `operator` (if any) an operator grant.
    pub async fn claimed(owner: &Identity, operator: Option<&Identity>) -> Self {
        let mut grants = BoundedVec::new();
        grants
            .push(OwnerGrant::from_retinue_identity(
                owner,
                ControllerRole::Owner,
            ))
            .unwrap();
        if let Some(operator) = operator {
            grants
                .push(OwnerGrant::from_retinue_identity(
                    operator,
                    ControllerRole::Operator,
                ))
                .unwrap();
        }
        let policy = RecoveryPolicy::new(
            RecoveryClause::new(ManagementCarrierSet::from_mask(1).unwrap(), 1).unwrap(),
            RecoveryClause::disabled(),
        )
        .unwrap();
        let state = DurableState::new(
            NODE,
            grants,
            ConfigGeneration(1),
            DurableConfig {
                public: initial_configuration(),
                sealed_credentials: BoundedVec::new(),
            },
            policy,
            &recovery_facts(),
        )
        .unwrap();
        let mut flash = Flash {
            a: vec![0xff; PAGE],
            b: vec![0xff; PAGE],
        };
        let mut body = [0_u8; MAX_DURABLE_BODY];
        let mut page = [0xff_u8; PAGE];
        let write = next_record(&flash.a, &flash.b, &state, &mut body, &mut page).unwrap();
        flash.program_slot(write.slot, &page[..write.len]).unwrap();
        let mut hardware = Hardware {
            flash,
            radio: Radio::default(),
        };
        // SAFETY: this in-process board is freshly constructed: there is no earlier radio,
        // flash or quiet state for the runtime to resume, which is what the reset proof covers.
        #[allow(unsafe_code)]
        let mut runtime = unsafe {
            ControlRuntime::new_after_hardware_reset(
                NODE,
                SemanticTagKey::from_bytes([0x80; 32]),
                recovery_facts(),
            )
        };
        let mut buffers = Box::new(([0; PAGE], [0; PAGE], [0; MAX_DURABLE_BODY], [0; PAGE]));
        let (x, y, b, p) = &mut *buffers;
        let mut scratch = DurableScratch::new(x, y, b, p).unwrap();
        let booted = runtime
            .boot_pre_radio(&mut hardware.flash, &mut hardware.radio, &mut scratch)
            .await
            .unwrap();
        assert_eq!(booted, BootState::Ready);
        Self {
            runtime,
            hardware,
            carrier: ControlCarrier::new(),
            buffers,
            start: Instant::now(),
            served: 0,
            interject: None,
            expiry_hold: Duration::ZERO,
        }
    }

    fn now_ms(start: Instant) -> u64 {
        start.elapsed().as_millis() as u64
    }

    /// When the board will roll the armed candidate back, as an instant on this host.
    pub fn deadline(&self) -> Option<Instant> {
        self.runtime
            .provisional_deadline_ms()
            .map(|ms| self.start + Duration::from_millis(ms) + self.expiry_hold)
    }

    /// Roll back a candidate whose board deadline (and hold) has passed, as the V4 loop does.
    pub async fn expire_if_due(&mut self) {
        let start = self.start;
        let hold = self.expiry_hold.as_millis() as u64;
        let (x, y, b, p) = &mut *self.buffers;
        let mut scratch = DurableScratch::new(x, y, b, p).unwrap();
        if self
            .runtime
            .provisional_deadline_ms()
            .is_some_and(|deadline| Self::now_ms(start) >= deadline + hold)
        {
            expire_provisional(&mut self.hardware, &mut self.runtime, &mut scratch, || {
                Self::now_ms(start)
            })
            .await;
        }
    }

    /// Serve one control frame through the shared carrier; return what it wrote back.
    pub async fn serve(&mut self, frame: &[u8]) -> std::vec::Vec<u8> {
        self.expire_if_due().await;
        let start = self.start;
        let first_write = FirstWriteStatus {
            control: PairEvidence::Valid,
            pending: PairEvidence::Blank,
        };
        let mut sink = Sink::default();
        {
            let (x, y, b, p) = &mut *self.buffers;
            let mut scratch = DurableScratch::new(x, y, b, p).unwrap();
            self.carrier
                .serve(
                    frame,
                    &mut self.hardware,
                    &mut self.runtime,
                    &mut scratch,
                    first_write,
                    || Self::now_ms(start),
                    &mut sink,
                )
                .await;
        }
        self.served += 1;
        if let Some((after, frames)) = self.interject.take() {
            if self.served == after {
                for frame in frames {
                    Box::pin(self.serve(&frame)).await;
                }
            } else {
                self.interject = Some((after, frames));
            }
        }
        sink.0
    }

    pub fn known_good(&self) -> (ConfigGeneration, PublicConfigurationV1) {
        let known_good = self.runtime.state().unwrap().known_good();
        (known_good.generation, known_good.configuration.public)
    }

    pub fn armed(&self) -> bool {
        self.runtime.state().unwrap().provisional().is_some()
    }

    /// The `applied` line for what the radio last applied. This board does not clamp power;
    /// its relay flags are the applied configuration's, as on a node.
    pub fn applied_line(&self) -> String {
        let public = self.hardware.radio.applied.last().expect("boot applied known-good");
        let transport = public.reticulum_transport();
        let relay = Relay::Claimed {
            announces: transport.relay_announces,
            packets: transport.relay_packets,
        };
        AppliedRadio::new(public.region(), &public.requested_reticulum_phy(), relay)
            .render()
            .as_str()
            .to_string()
    }
}

/// Run the board on the board end of a duplex: deframe KISS command frames, serve each, answer
/// `applied` lines outside frames, and roll back at the deadline without waiting for a frame.
pub async fn run(board: Arc<Mutex<Board>>, mut io: DuplexStream) {
    let mut deframer = selvage::kiss::Deframer::<MAX_CONTROL_COMMAND_FRAME_LEN>::new();
    let mut bytes = [0_u8; 256];
    let mut in_frame = false;
    let mut text = std::vec::Vec::new();
    loop {
        let deadline = board.lock().await.deadline();
        let wake = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            read = io.read(&mut bytes) => {
                let Ok(count) = read else { return };
                if count == 0 {
                    return;
                }
                for &byte in &bytes[..count] {
                    if byte == selvage::kiss::FEND {
                        in_frame = !in_frame;
                        text.clear();
                    } else if !in_frame {
                        if byte == b'\n' {
                            if radio_hand::applied::is_query(&[&text[..], b"\n"].concat()) {
                                let line = board.lock().await.applied_line();
                                if io.write_all(line.as_bytes()).await.is_err() {
                                    return;
                                }
                            }
                            text.clear();
                        } else {
                            text.push(byte);
                        }
                    }
                    if deframer.push(byte) {
                        let frame = deframer.frame().to_vec();
                        let reply = board.lock().await.serve(&frame).await;
                        if io.write_all(&reply).await.is_err() {
                            return;
                        }
                    }
                }
            }
            () = wake => board.lock().await.expire_if_due().await,
        }
    }
}

pub const fn token() -> [u8; COMMIT_TOKEN_LEN] {
    [TOKEN_BYTE; COMMIT_TOKEN_LEN]
}
