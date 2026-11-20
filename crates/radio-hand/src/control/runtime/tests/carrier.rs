//! The shared carrier's ordering, against a fake board that logs what a V4 owner would do.
//!
//! The board stands in for the V4 radio owner: a quiet preflight, a true-entropy source, and a
//! quiet window whose entry stops the radio, whose guard journals and applies, and whose end is
//! exactly one receive restore or one reset (a dropped guard or entry resets, as the V4 does).

use core::cell::RefCell;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Waker};
use std::rc::Rc;
use std::vec::Vec as StdVec;

use super::fakes::*;
use crate::control::carrier::{ControlBoard, ControlCarrier, NoEntropy, expire_provisional};
use crate::control::*;
use crate::link::{HostLink, LinkFault};
use crate::store::Slot;
use futures::executor::block_on;
use retinue::identity::PrivateIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ev {
    Preflight,
    Entropy,
    Enter,
    Store,
    Apply,
    Restore,
    Reset,
}

#[derive(Clone, Default)]
pub(super) struct Log(Rc<RefCell<StdVec<Ev>>>);

impl Log {
    fn push(&self, ev: Ev) {
        self.0.borrow_mut().push(ev);
    }
    pub(super) fn take(&self) -> StdVec<Ev> {
        core::mem::take(&mut *self.0.borrow_mut())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pend {
    Entry,
    Apply,
    Finish,
}

/// A V4-shaped board. `rx_owed` is the owner's `prepare_rx`: set by a send or a quiet entry,
/// cleared by arming receive or by a completed quiet finish, which re-arms receive.
pub(super) struct Board {
    pub(super) log: Log,
    store: FakeStore,
    applier: FakeApplier,
    pub(super) ready: bool,
    pub(super) entropy: bool,
    pub(super) refuse_entry: bool,
    pub(super) pend: Option<Pend>,
    pub(super) rx_owed: bool,
    pub(super) reset: bool,
}

struct EntryReset(Log);

impl Drop for EntryReset {
    fn drop(&mut self) {
        self.0.push(Ev::Reset);
    }
}

pub(super) struct Guard<'a> {
    board: &'a mut Board,
    done: bool,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.board.log.push(Ev::Reset);
            self.board.reset = true;
        }
    }
}

impl QuietWindow for Board {
    type Error = QuietFailure;
    type StoreError = StoreFailure;
    type ApplyError = ApplyFailure;
    type Guard<'a>
        = Guard<'a>
    where
        Self: 'a;

    async fn enter(&mut self) -> Result<Guard<'_>, QuietFailure> {
        // Refused before stopping begins: ordinary operation is untouched.
        if self.refuse_entry {
            return Err(QuietFailure::Enter);
        }
        self.log.push(Ev::Enter);
        self.rx_owed = true;
        if self.pend == Some(Pend::Entry) {
            let _reset = EntryReset(self.log.clone());
            core::future::pending::<()>().await;
        }
        Ok(Guard {
            board: self,
            done: false,
        })
    }
}

impl QuietGuard for Guard<'_> {
    type Error = QuietFailure;

    fn abort(&mut self) {
        if !self.done {
            self.done = true;
            self.board.log.push(Ev::Reset);
            self.board.reset = true;
        }
    }

    async fn finish(&mut self) -> Result<QuietExit, QuietFailure> {
        if self.board.pend == Some(Pend::Finish) {
            core::future::pending::<()>().await;
        }
        self.board.log.push(Ev::Restore);
        self.board.rx_owed = false;
        self.done = true;
        Ok(QuietExit::Resumed)
    }
}

impl AbSlotStore for Guard<'_> {
    type Error = StoreFailure;

    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> Result<(), Self::Error> {
        self.board.store.read_slot(slot, out)
    }

    fn erase_slot(&mut self, slot: Slot) -> Result<(), Self::Error> {
        self.board.store.erase_slot(slot)
    }

    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> Result<(), Self::Error> {
        self.board.log.push(Ev::Store);
        self.board.store.program_slot(slot, record)
    }
}

impl ConfigApplier for Guard<'_> {
    type Error = ApplyFailure;

    async fn apply(&mut self, configuration: &DurableConfig) -> Result<(), Self::Error> {
        self.board.log.push(Ev::Apply);
        if self.board.pend == Some(Pend::Apply) {
            core::future::pending::<()>().await;
        }
        self.board.applier.apply(configuration).await
    }
}

impl ControlBoard for Board {
    fn quiet_ready(&self) -> bool {
        self.log.push(Ev::Preflight);
        self.ready && !self.rx_owed && !self.reset
    }

    fn commit_entropy(&mut self, out: &mut [u8]) -> Result<(), NoEntropy> {
        self.log.push(Ev::Entropy);
        if self.entropy {
            out.fill(0xA5);
            Ok(())
        } else {
            Err(NoEntropy)
        }
    }
}

#[derive(Default)]
pub(super) struct Host {
    pub(super) written: StdVec<u8>,
}

impl HostLink for Host {
    async fn attached(&mut self) {}

    async fn read(&mut self, _buf: &mut [u8]) -> Result<usize, LinkFault> {
        core::future::pending().await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), LinkFault> {
        self.written.extend_from_slice(bytes);
        Ok(())
    }
}

/// A booted runtime over a claimed journal, a fake board, and the carrier under test.
pub(super) struct Rig {
    pub(super) board: Board,
    pub(super) runtime: ControlRuntime,
    pub(super) carrier: ControlCarrier,
    pub(super) host: Host,
    pub(super) operator: PrivateIdentity,
    x: [u8; PAGE],
    y: [u8; PAGE],
    b: [u8; MAX_DURABLE_BODY],
    p: [u8; PAGE],
}

impl Rig {
    pub(super) fn new() -> Self {
        let t = Trace::default();
        let operator = operator();
        let mut store = FakeStore::blank(&t);
        seed(&mut store, &state_for_operator(&operator));
        let mut applier = FakeApplier::new(&t);
        let (mut x, mut y, mut b, mut p) = super::buffers();
        let mut runtime = super::runtime();
        block_on(runtime.boot_pre_radio(
            &mut store,
            &mut applier,
            &mut scratch(&mut x, &mut y, &mut b, &mut p),
        ))
        .unwrap();
        Self {
            board: Board {
                log: Log::default(),
                store,
                applier,
                ready: true,
                entropy: true,
                refuse_entry: false,
                pend: None,
                rx_owed: false,
                reset: false,
            },
            runtime,
            carrier: ControlCarrier::new(),
            host: Host::default(),
            operator,
            x,
            y,
            b,
            p,
        }
    }

    /// One provisional apply, signed by `signer` with outer `counter`, as a carrier frame.
    pub(super) fn apply_frame(
        signer: &PrivateIdentity,
        counter: u64,
        lifetime_ms: u64,
    ) -> StdVec<u8> {
        let arguments = ProvisionalApplyArguments {
            change: ChangeId([0x77; 16]),
            public: configuration(b"candidate").public,
            lifetime_ms,
        };
        let request = Request {
            transaction: TransactionId([counter as u8; 16]),
            transaction_sequence: counter,
            expected_generation: ConfigGeneration(7),
            operation: Operation::ProvisionalApply,
            arguments: heapless::Vec::try_from(arguments.encode().as_slice()).unwrap(),
        };
        let mut frame = std::vec![CONTROL_COMMAND_FRAME_TAG];
        frame.extend_from_slice(&signed_command(signer, &encoded_request(&request), counter));
        frame
    }

    fn first_write() -> FirstWriteStatus {
        FirstWriteStatus {
            control: PairEvidence::Valid,
            pending: PairEvidence::Blank,
        }
    }

    /// The carrier's serve future for `frame`, at board time 1,000 ms.
    pub(super) fn serve<'a>(&'a mut self, frame: &'a [u8]) -> impl Future<Output = ()> + 'a {
        let mut scratch = scratch(&mut self.x, &mut self.y, &mut self.b, &mut self.p);
        let carrier = &mut self.carrier;
        let board = &mut self.board;
        let runtime = &mut self.runtime;
        let host = &mut self.host;
        async move {
            carrier
                .serve(
                    frame,
                    board,
                    runtime,
                    &mut scratch,
                    Self::first_write(),
                    || 1_000,
                    host,
                )
                .await
        }
    }

    pub(super) fn expire(&mut self, now: u64) -> bool {
        let mut scratch = scratch(&mut self.x, &mut self.y, &mut self.b, &mut self.p);
        block_on(expire_provisional(
            &mut self.board,
            &mut self.runtime,
            &mut scratch,
            || now,
        ))
    }
}

/// Poll a future once and drop it, as a cancelled `select` branch would.
pub(super) fn poll_once_and_drop<F: Future>(future: F) -> bool {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    future.as_mut().poll(&mut context).is_pending()
}

fn count(log: &[Ev], ev: Ev) -> usize {
    log.iter().filter(|e| **e == ev).count()
}

#[test]
fn quiet_preflight_is_asked_before_the_verifier_advances() {
    let mut rig = Rig::new();
    let frame = Rig::apply_frame(&rig.operator, 1, 60_000);
    rig.board.ready = false;
    block_on(rig.serve(&frame));
    assert!(rig.host.written.is_empty(), "a refusal is silent");
    assert_eq!(rig.board.log.take(), [Ev::Preflight]);

    // The same counter is still fresh: the verifier never saw the first frame.
    rig.board.ready = true;
    block_on(rig.serve(&frame));
    assert!(!rig.host.written.is_empty());
    let log = rig.board.log.take();
    assert_eq!(&log[..3], [Ev::Preflight, Ev::Entropy, Ev::Enter]);
    assert_eq!(log.last(), Some(&Ev::Restore));
}

#[test]
fn commit_token_is_drawn_before_verification_and_an_entropy_fault_costs_nothing() {
    let mut rig = Rig::new();
    let stranger = PrivateIdentity::from_secret_bytes(&[0x22; 64]);
    // Drawn before verification: an unverifiable frame still costs a draw, nothing more.
    block_on(rig.serve(&Rig::apply_frame(&stranger, 1, 60_000)));
    assert_eq!(rig.board.log.take(), [Ev::Preflight, Ev::Entropy]);

    let frame = Rig::apply_frame(&rig.operator, 1, 60_000);
    rig.board.entropy = false;
    block_on(rig.serve(&frame));
    assert!(rig.host.written.is_empty());
    assert_eq!(rig.board.log.take(), [Ev::Preflight, Ev::Entropy]);

    // The verifier did not advance, so the same counter is accepted once entropy returns.
    rig.board.entropy = true;
    block_on(rig.serve(&frame));
    assert!(!rig.host.written.is_empty());
}

#[test]
fn a_runtime_error_discards_the_verifier_and_the_next_frame_rebuilds_it() {
    let mut rig = Rig::new();
    let frame = Rig::apply_frame(&rig.operator, 1, 60_000);
    // Verified, then the quiet window refuses before stopping: the in-memory verifier has
    // accepted counter 1 but nothing was journaled.
    rig.board.refuse_entry = true;
    block_on(rig.serve(&frame));
    assert!(rig.host.written.is_empty());
    assert_eq!(count(&rig.board.log.take(), Ev::Enter), 0);

    // Rebuilt from durable state, which still expects counter 1.
    rig.board.refuse_entry = false;
    block_on(rig.serve(&frame));
    assert!(!rig.host.written.is_empty());
    assert_eq!(
        rig.runtime.state().unwrap().owner_grants()[0].accepted_outer_counter(),
        1
    );
}

#[test]
fn refusals_are_silent() {
    let mut rig = Rig::new();
    let accepted = Rig::apply_frame(&rig.operator, 1, 60_000);
    block_on(rig.serve(&accepted));
    let answered = rig.host.written.len();
    assert!(answered > 0);
    rig.board.log.take();

    let stranger = PrivateIdentity::from_secret_bytes(&[0x22; 64]);
    let mut tampered = Rig::apply_frame(&rig.operator, 2, 60_000);
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    for frame in [
        std::vec![CONTROL_COMMAND_FRAME_TAG],
        std::vec![0x00, 1, 2, 3],
        accepted.clone(),
        Rig::apply_frame(&stranger, 2, 60_000),
        tampered,
    ] {
        block_on(rig.serve(&frame));
    }
    assert_eq!(rig.host.written.len(), answered, "no refusal earns a reply");
    assert_eq!(count(&rig.board.log.take(), Ev::Enter), 0);
}

#[test]
fn a_dropped_serve_resets_exactly_once_at_entry_apply_and_finish() {
    for pend in [Pend::Entry, Pend::Apply, Pend::Finish] {
        let mut rig = Rig::new();
        let frame = Rig::apply_frame(&rig.operator, 1, 60_000);
        rig.board.pend = Some(pend);
        assert!(poll_once_and_drop(rig.serve(&frame)), "{pend:?} pends");
        let log = rig.board.log.take();
        assert_eq!(count(&log, Ev::Enter), 1, "{pend:?}");
        assert_eq!(count(&log, Ev::Reset), 1, "{pend:?}: one reset");
        assert_eq!(count(&log, Ev::Restore), 0, "{pend:?}: no restore");
        assert!(rig.host.written.is_empty(), "{pend:?}: nothing answered");
    }
}

#[test]
fn expiry_waits_for_a_quiet_boundary_then_rolls_back() {
    let mut rig = Rig::new();
    block_on(rig.serve(&Rig::apply_frame(&rig.operator, 1, 1_000)));
    assert_eq!(rig.runtime.provisional_deadline_ms(), Some(2_000));
    rig.board.log.take();

    rig.board.ready = false;
    assert!(!rig.expire(2_000), "not at a quiet boundary yet");
    assert_eq!(rig.board.log.take(), [Ev::Preflight]);

    rig.board.ready = true;
    assert!(rig.expire(2_000));
    let log = rig.board.log.take();
    assert_eq!(&log[..2], [Ev::Preflight, Ev::Enter]);
    assert!(
        log.contains(&Ev::Apply),
        "known-good goes back on the radio"
    );
    assert_eq!(log.last(), Some(&Ev::Restore));
    assert!(rig.runtime.provisional_deadline_ms().is_none());
}
