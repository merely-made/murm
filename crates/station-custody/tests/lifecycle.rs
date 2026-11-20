//! The station-claim lifecycle against an in-process board running the real control runtime
//! and shared carrier behind the literal USB framing. A fake authority (an in-memory persona)
//! stands in for djinn; no serial port, no djinn and no real authority root are touched.

#[path = "lifecycle/board.rs"]
mod board;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use board::{Board, NODE, initial_configuration, token};
use personae::InMemoryProvider;
use personae::reticulum::ReticulumControllerMaterial;
use postilion::control::sign_request;
use postilion::control::verified::{UsbControlConfig, UsbControlTransport};
use radio_hand::control::{
    ChangeId, CommitArguments, ConfigGeneration, ManagementCarrierSet, Operation,
    ProvisionalApplyArguments, PublicConfigurationV1, Request, ReticulumTransportPolicy,
    TransactionId, encode_command_frame,
};
use radio_hand::region::Region;
use retinue::hash::AddressHash;
use retinue::identity::PrivateIdentity;
use station_custody::lifecycle::{
    FileControllerState, InMemoryTokenHolder, LifecycleFailure, LifecyclePlan, LifecycleRecord,
    Then, run_lifecycle,
};
use tokio::sync::{Mutex, Notify};

const SCOPE: &[u8] = b"first-owner-controller:test-scope";

fn provider() -> InMemoryProvider {
    InMemoryProvider::from_seed([0x4a; 32])
}

/// The controller identity the fake authority releases for `SCOPE`, for the board's grant.
fn controller() -> PrivateIdentity {
    let material = ReticulumControllerMaterial::derive(&provider(), SCOPE).unwrap();
    PrivateIdentity::from_secret_bytes(&material.secret_bytes())
}

/// Relay on, sync 0x12, every radio field away from the board's boot profile.
fn candidate() -> PublicConfigurationV1 {
    let mut phy = selvage::PhyProfile::meshtastic_long_fast(904_375_000);
    phy.bandwidth_hz = 125_000;
    phy.spreading_factor = 9;
    phy.coding_rate_denominator = 6;
    phy.preamble_symbols = 12;
    phy.sync_word = 0x12;
    phy.tx_power_dbm = 14;
    PublicConfigurationV1::new(
        Region::Us915,
        phy,
        ReticulumTransportPolicy::new(true, true, 8).unwrap(),
        ManagementCarrierSet::from_mask(1).unwrap(),
    )
    .unwrap()
}

fn plan(then: Then, lifetime_ms: u64) -> LifecyclePlan {
    LifecyclePlan {
        node: NODE,
        candidate: candidate(),
        lifetime_ms,
        then,
        expiry_margin: Duration::from_millis(300),
    }
}

struct Bench {
    board: Arc<Mutex<Board>>,
    root: tempfile::TempDir,
    lines: Vec<String>,
}

async fn bench(operator: Option<&PrivateIdentity>) -> Bench {
    let board = Board::claimed(controller().public(), operator.map(PrivateIdentity::public)).await;
    Bench {
        board: Arc::new(Mutex::new(board)),
        root: tempfile::tempdir().unwrap(),
        lines: Vec::new(),
    }
}

impl Bench {
    /// The tool's carrier, and the board serving the other end until the tool lets go.
    fn transport(
        &self,
    ) -> (
        UsbControlTransport<tokio::io::DuplexStream>,
        impl std::future::Future<Output = ()> + use<>,
    ) {
        let (tool, board_end) = tokio::io::duplex(4096);
        let transport = UsbControlTransport::from_io(
            tool,
            UsbControlConfig {
                response_timeout: Duration::from_millis(500),
                ..UsbControlConfig::default()
            },
        );
        (transport, board::run(Arc::clone(&self.board), board_end))
    }

    async fn run(&mut self, plan: &LifecyclePlan) -> Result<LifecycleRecord, LifecycleFailure> {
        let (transport, board) = self.transport();
        let mut state = FileControllerState::new(self.root.path());
        let mut holder = InMemoryTokenHolder::new();
        let lines = &mut self.lines;
        let mut report = |line: &str| lines.push(line.to_string());
        let provider = provider();
        let tool = run_lifecycle(
            &provider,
            SCOPE,
            transport,
            plan,
            &mut state,
            &mut holder,
            &mut report,
        );
        let (result, ()) = tokio::join!(tool, board);
        assert!(!holder.holds(), "no token outlives the run");
        result
    }

    fn spent(&self, file: &str) -> u64 {
        let text = std::fs::read_to_string(self.root.path().join(file)).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["last_used"]
            .as_u64()
            .unwrap()
    }
}

const COUNTER: &str = "first-owner-controller-counter.json";
const SEQUENCE: &str = "first-owner-controller-sequence.json";

/// The token, raw or as hex in either case, appears nowhere in these bytes.
fn assert_no_token(what: &str, bytes: &[u8]) {
    let raw = token();
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    assert!(
        !bytes.windows(raw.len()).any(|w| w == raw),
        "{what} holds the raw token"
    );
    let text = String::from_utf8_lossy(bytes).to_lowercase();
    assert!(!text.contains(&hex), "{what} holds the token as hex");
}

fn assert_no_token_in_files(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        assert_no_token(&path.display().to_string(), &std::fs::read(&path).unwrap());
    }
}

#[tokio::test]
async fn apply_then_commit_moves_known_good_and_leaks_no_token() {
    let mut bench = bench(None).await;
    let record = bench.run(&plan(Then::Commit, 60_000)).await.unwrap();

    assert_eq!(record.status_before.unwrap().known_good_generation, 1);
    assert_eq!(record.candidate_generation, Some(2));
    assert_eq!(record.status_during.unwrap().generation_watermark, 2);
    assert_eq!(record.known_good_after, Some(2));
    let counters: Vec<u64> = record.steps.iter().map(|s| s.counter).collect();
    assert_eq!(counters, [1, 2, 3, 4], "status, apply, status, commit");
    let sequences: Vec<u64> = record.steps.iter().filter_map(|s| s.sequence).collect();
    assert_eq!(sequences, [1, 2], "apply and commit each spend one");
    let board = bench.board.lock().await;
    assert_eq!(board.known_good(), (ConfigGeneration(2), candidate()));
    assert!(!board.armed());
    drop(board);
    assert_eq!((bench.spent(COUNTER), bench.spent(SEQUENCE)), (4, 2));

    // The search finds the token when it is there, raw or as hex.
    assert!(std::panic::catch_unwind(|| assert_no_token("control", &token())).is_err());
    assert!(
        std::panic::catch_unwind(|| assert_no_token(
            "control",
            b"xx c3C3c3c3c3c3c3c3c3c3c3c3c3c3c3c3 xx"
        ))
        .is_err()
    );
    let json = serde_json::to_vec(&record).unwrap();
    assert_no_token("record", &json);
    assert_no_token("output", bench.lines.join("\n").as_bytes());
    assert_no_token_in_files(bench.root.path());
}

#[tokio::test]
async fn a_relay_on_sync_0x12_candidate_reaches_the_board_radio_exactly() {
    let mut bench = bench(None).await;
    let record = bench.run(&plan(Then::Commit, 60_000)).await.unwrap();
    let board = bench.board.lock().await;
    let applied = &board.hardware.radio.applied;
    assert_eq!(
        applied.first(),
        Some(&initial_configuration()),
        "boot applied known-good"
    );
    assert_eq!(
        applied.last(),
        Some(&candidate()),
        "the candidate, every field"
    );
    let sent = &record.candidate;
    assert!(sent.relay_announces && sent.relay_packets);
    assert_eq!((sent.sync_word.as_str(), sent.max_hops), ("0x12", 8));
    assert_eq!(
        record.applied_reported, None,
        "the board's answer carries no configuration"
    );
}

#[tokio::test]
async fn apply_then_revert_restores_known_good() {
    let mut bench = bench(None).await;
    let record = bench.run(&plan(Then::Revert, 60_000)).await.unwrap();
    assert_eq!(record.known_good_after, Some(1));
    let board = bench.board.lock().await;
    assert_eq!(
        board.known_good(),
        (ConfigGeneration(1), initial_configuration())
    );
    assert!(!board.armed());
    assert_eq!(
        board.hardware.radio.applied.last(),
        Some(&initial_configuration())
    );
    drop(board);
    assert_no_token("output", bench.lines.join("\n").as_bytes());
}

#[tokio::test]
async fn apply_then_expiry_rolls_back_on_the_board_and_status_is_read_after() {
    let mut bench = bench(None).await;
    let record = bench.run(&plan(Then::Expire, 1_000)).await.unwrap();
    let after = record.status_after.expect("status read after the deadline");
    assert_eq!(after.known_good_generation, 1);
    assert_eq!(record.known_good_after, Some(1));
    let counters: Vec<u64> = record.steps.iter().map(|s| s.counter).collect();
    assert_eq!(counters, [1, 2, 3, 4], "nothing sent during the wait");
    let board = bench.board.lock().await;
    assert!(
        !board.armed(),
        "the board rolled the candidate back by itself"
    );
    assert_eq!(
        board.hardware.radio.applied.last(),
        Some(&initial_configuration())
    );
}

#[tokio::test]
async fn a_replayed_counter_earns_silence_and_the_record_is_not_rewound() {
    let mut bench = bench(None).await;
    bench.run(&plan(Then::Revert, 60_000)).await.unwrap();
    // Roll the counter record back behind the board, as a restored backup would.
    std::fs::write(bench.root.path().join(COUNTER), br#"{"last_used":1}"#).unwrap();
    let failure = bench.run(&plan(Then::Commit, 60_000)).await.unwrap_err();
    assert_eq!(failure.step, "status-before");
    assert!(
        failure.message.contains("answered nothing"),
        "{}",
        failure.message
    );
    assert!(
        failure.message.contains("counter record"),
        "{}",
        failure.message
    );
    // The spent counter stays spent; the sequence was never touched.
    assert_eq!((bench.spent(COUNTER), bench.spent(SEQUENCE)), (2, 2));
    assert!(!bench.board.lock().await.armed());
}

/// Another controller's signed frame, for interjecting between the tool's steps.
fn signed_frame(signer: &PrivateIdentity, counter: u64, request: &Request) -> Vec<u8> {
    let wire = sign_request(request, signer, AddressHash::from_bytes(NODE.0), counter).unwrap();
    let mut frame = vec![0_u8; radio_hand::control::MAX_CONTROL_COMMAND_FRAME_LEN];
    let len = encode_command_frame(&wire, &mut frame).unwrap();
    frame.truncate(len);
    frame
}

fn operator_change(operator: &PrivateIdentity) -> Vec<Vec<u8>> {
    let change = ChangeId([0x0e; 16]);
    let apply = ProvisionalApplyArguments {
        change,
        public: initial_configuration(),
        lifetime_ms: 60_000,
    }
    .encode();
    let commit = CommitArguments {
        change,
        candidate_generation: ConfigGeneration(2),
        commit_token: token(),
    }
    .encode();
    let request = |sequence, operation, arguments: &[u8]| Request {
        transaction: TransactionId([sequence as u8; 16]),
        transaction_sequence: sequence,
        expected_generation: ConfigGeneration(1),
        operation,
        arguments: heapless::Vec::try_from(arguments).unwrap(),
    };
    vec![
        signed_frame(
            operator,
            1,
            &request(1, Operation::ProvisionalApply, &apply),
        ),
        signed_frame(operator, 2, &request(2, Operation::Commit, &commit)),
    ]
}

#[tokio::test]
async fn a_stale_generation_is_refused_and_the_next_invocation_succeeds() {
    let operator = PrivateIdentity::from_secret_bytes(&[0x0b; 64]);
    let mut bench = bench(Some(&operator)).await;
    // Between the tool's status read and its apply, an operator commits generation 2.
    bench.board.lock().await.interject = Some((1, operator_change(&operator)));
    let failure = bench.run(&plan(Then::Commit, 60_000)).await.unwrap_err();
    assert_eq!(failure.step, "apply");
    assert!(
        failure.message.contains("StaleGeneration"),
        "{}",
        failure.message
    );
    assert_eq!((bench.spent(COUNTER), bench.spent(SEQUENCE)), (2, 1));
    assert_eq!(bench.board.lock().await.known_good().0, ConfigGeneration(2));

    let record = bench.run(&plan(Then::Commit, 60_000)).await.unwrap();
    assert_eq!(record.status_before.unwrap().known_good_generation, 2);
    assert_eq!(record.known_good_after, Some(3));
    assert_eq!(
        record.steps[0].counter, 3,
        "the next counter after the spent ones"
    );
}

#[tokio::test]
async fn dying_between_apply_and_commit_leaves_a_candidate_that_expires_then_runs_again() {
    let mut bench = bench(None).await;
    let applied = Notify::new();
    {
        let (transport, board) = bench.transport();
        let mut state = FileControllerState::new(bench.root.path());
        let mut holder = InMemoryTokenHolder::new();
        let plan = plan(Then::Commit, 1_000);
        let provider = provider();
        let mut report = |line: &str| {
            if line.starts_with("applied") {
                applied.notify_one();
            }
        };
        let run = run_lifecycle(
            &provider,
            SCOPE,
            transport,
            &plan,
            &mut state,
            &mut holder,
            &mut report,
        );
        // The process dies at its next await after the apply answered; dropping the tool's
        // end of the carrier lets the board loop finish.
        let tool = async {
            tokio::select! {
                _ = run => panic!("the run must not finish"),
                () = applied.notified() => {}
            }
        };
        tokio::join!(tool, board);
    }
    assert!(
        bench.board.lock().await.armed(),
        "the candidate is left armed"
    );
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    // What the board's own deadline timer does, with nothing on its host link.
    bench.board.lock().await.expire_if_due().await;
    assert!(
        !bench.board.lock().await.armed(),
        "the board's deadline rolled it back"
    );
    assert_eq!(bench.board.lock().await.known_good().0, ConfigGeneration(1));

    let record = bench.run(&plan(Then::Commit, 60_000)).await.unwrap();
    assert_eq!(
        record.known_good_after,
        Some(3),
        "generation 2 was the expired candidate"
    );
    assert!(record.steps[0].counter > 2, "no counter is offered twice");
}
