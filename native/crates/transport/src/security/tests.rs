// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

fn keys(seed: u8) -> Ed25519Keypair {
    Ed25519Keypair::from_seed([seed; 32])
}
fn client_key() -> &'static Ed25519Keypair {
    static KEY: std::sync::OnceLock<Ed25519Keypair> = std::sync::OnceLock::new();
    KEY.get_or_init(|| keys(1))
}
fn server_key() -> &'static Ed25519Keypair {
    static KEY: std::sync::OnceLock<Ed25519Keypair> = std::sync::OnceLock::new();
    KEY.get_or_init(|| keys(2))
}
fn protocol() -> Alpn {
    Alpn::new("mere/bounded-setup/v1")
}
fn policy() -> ServiceSecurityPolicy {
    ServiceSecurityPolicy::noise(protocol(), None, SetupLimits::default())
}

#[test]
fn hard_ceilings_refuse_unbounded_configuration_without_semaphore_panics() {
    let gate = SetupGate::new(usize::MAX);
    assert_eq!(gate.limit, MAX_PENDING_SETUPS);
    for limits in [
        SetupLimits {
            max_read_bytes: MAX_SETUP_BYTES + 1,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_write_bytes: MAX_SETUP_BYTES + 1,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_frame_bytes: 65536,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_frames: MAX_SETUP_FRAMES + 1,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_io_operations: MAX_SETUP_IO_OPERATIONS + 1,
            ..SetupLimits::default()
        },
    ] {
        assert!(matches!(
            gate.reserve(&ServiceSecurityPolicy::noise(protocol(), None, limits)),
            Err(SetupError::InvalidPolicy)
        ));
    }
    let reservation = gate.reserve(&policy()).unwrap();
    assert!(
        matches!(reservation.admitted(), Err(SetupError::Closed)),
        "a reservation cannot complete before security"
    );
    assert_eq!(gate.pending(), 0);
}

async fn pair(
    gate: &SetupGate,
    policy: &ServiceSecurityPolicy,
) -> (
    (
        PreparedStream<DuplexStream>,
        SessionSecurityFacts,
        SetupGuard,
    ),
    (
        PreparedStream<DuplexStream>,
        SessionSecurityFacts,
        SetupGuard,
    ),
) {
    let (left, right) = tokio::io::duplex(8192);
    let (left, right) = tokio::join!(
        gate.noise_initiator(client_key(), left, policy),
        gate.noise_responder(server_key(), right, policy)
    );
    (left.unwrap(), right.unwrap())
}

#[tokio::test]
async fn capacity_is_retained_through_admission_and_released_on_every_terminal_path() {
    let gate = SetupGate::new(2);
    let ((mut left, _, left_guard), (mut right, _, right_guard)) = pair(&gate, &policy()).await;
    assert_eq!(gate.pending(), 2);
    assert!(matches!(gate.reserve(&policy()), Err(SetupError::Capacity)));
    left_guard.run(async {}).await.unwrap();
    assert_eq!(
        gate.pending(),
        2,
        "successful security/run does not release admission capacity"
    );
    left_guard.admitted().unwrap();
    right_guard.admitted().unwrap();
    assert_eq!(gate.pending(), 0);
    // After admission the application owns its own budgets and wire framing.
    // This frame exceeds the setup cap but remains a legal legacy Noise frame.
    let bytes = vec![42; 8192];
    let mut read = vec![0; bytes.len()];
    let (write, read_result) = tokio::join!(
        async {
            left.write_all(&bytes).await?;
            left.flush().await
        },
        right.read_exact(&mut read)
    );
    write.unwrap();
    read_result.unwrap();
    assert_eq!(read, bytes);
    assert_eq!(gate.stats().admitted, 2);
}

#[tokio::test]
async fn dropping_the_guard_poisons_already_buffered_plaintext() {
    let gate = SetupGate::new(2);
    let ((mut left, _, left_guard), (mut right, _, right_guard)) = pair(&gate, &policy()).await;
    let framed = [0, 0, 0, 4, 1, 2, 3, 4];
    left.write_all(&framed).await.unwrap();
    left.flush().await.unwrap();
    let mut prefix = [0; 4];
    right.read_exact(&mut prefix).await.unwrap();
    drop(right_guard);
    assert!(
        right.read_u8().await.is_err(),
        "buffered plaintext is still behind the poisoned outer stream"
    );
    drop(left_guard);
    assert_eq!(gate.pending(), 0);
    assert_eq!(gate.stats().cancelled, 2);
}

#[tokio::test]
async fn cancelling_only_run_poisons_partial_framing_and_releases_the_retained_lease() {
    let gate = SetupGate::new(1);
    let public = ServiceSecurityPolicy::public_plaintext(protocol(), SetupLimits::default());
    let (inner, mut peer) = tokio::io::duplex(1024);
    let (mut stream, _, guard) = gate
        .reserve(&public)
        .unwrap()
        .public_plaintext(inner)
        .unwrap();
    // A valid frame prefix with only part of its body. The read consumes six
    // bytes and stalls; cancelling it must not permit a fresh framing attempt.
    peer.write_all(&[0, 0, 0, 8, 11, 12]).await.unwrap();
    let mut bytes = [0; 12];
    let mut running = Box::pin(guard.run(stream.read_exact(&mut bytes)));
    tokio::select! {
        result = &mut running => panic!("partial frame completed: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(5)) => {},
    }
    drop(running);
    assert_eq!(&bytes[..6], &[0, 0, 0, 8, 11, 12]);
    assert_eq!(
        gate.pending(),
        0,
        "cancelled run returns capacity while guard and stream remain retained"
    );
    assert!(stream.read_u8().await.is_err());
    assert!(stream.write_all(&[0, 0, 0, 1, 5]).await.is_err());
    assert!(matches!(guard.run(async {}).await, Err(SetupError::Closed)));
    assert!(matches!(guard.admitted(), Err(SetupError::Closed)));
    assert_eq!(gate.stats().cancelled, 1);
}

#[tokio::test]
async fn cancelling_only_run_also_blocks_cached_noise_plaintext() {
    let gate = SetupGate::new(2);
    let ((mut left, _, left_guard), (mut right, _, right_guard)) = pair(&gate, &policy()).await;
    left.write_all(&[0, 0, 0, 4, 11, 12, 13, 14]).await.unwrap();
    left.flush().await.unwrap();
    let mut prefix = [0; 4];
    right_guard
        .run(right.read_exact(&mut prefix))
        .await
        .unwrap()
        .unwrap();
    let mut consumed = [0; 2];
    let partial = async {
        right.read_exact(&mut consumed).await.unwrap();
        std::future::pending::<()>().await
    };
    let mut running = Box::pin(right_guard.run(partial));
    tokio::select! {
        result = &mut running => panic!("unfinished admission completed: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(5)) => {},
    }
    drop(running);
    assert_eq!(consumed, [11, 12]);
    assert_eq!(gate.pending(), 1, "only the unrelated peer's lease remains");
    assert!(
        right.read_u8().await.is_err(),
        "remaining cached plaintext cannot escape"
    );
    assert!(right.write_all(b"no further protocol bytes").await.is_err());
    assert!(matches!(right_guard.admitted(), Err(SetupError::Closed)));
    left_guard.refused();
    assert_eq!(gate.pending(), 0);
}

#[tokio::test]
async fn a_security_lease_cannot_be_nested_or_restarted_after_its_handshake() {
    let gate = SetupGate::new(2);
    let ((left, _, left_guard), (_, _, right_guard)) = pair(&gate, &policy()).await;
    let repeated = left_guard.noise_initiator(client_key(), left).await;
    assert!(matches!(repeated, Err(SetupError::Closed)));
    drop(right_guard);
    assert_eq!(gate.pending(), 0);
}

#[tokio::test]
async fn cancelling_a_stalled_handshake_releases_the_shared_permit() {
    let gate = SetupGate::new(1);
    let task_gate = gate.clone();
    let (_silent, stream) = tokio::io::duplex(1024);
    let task = tokio::spawn(async move {
        task_gate
            .noise_responder(server_key(), stream, &policy())
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while gate.pending() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert_eq!(gate.pending(), 0);
    assert_eq!(gate.stats().cancelled, 1);
    assert!(gate.reserve(&policy()).is_ok());
}

#[tokio::test]
async fn one_absolute_deadline_covers_establishment_and_later_admission() {
    let gate = SetupGate::new(1);
    let limits = SetupLimits {
        timeout: Duration::from_millis(20),
        ..SetupLimits::default()
    };
    let public = ServiceSecurityPolicy::public_plaintext(protocol(), limits);
    let guard = gate.reserve(&public).unwrap();
    guard.run(async {}).await.unwrap();
    let (stream, _peer) = tokio::io::duplex(1024);
    let (mut stream, facts, guard) = guard.public_plaintext(stream).unwrap();
    assert_eq!(facts.peer(), None);
    let result = guard.run(std::future::pending::<()>()).await;
    assert!(matches!(
        result,
        Err(SetupError::Deadline {
            stage: SetupStage::Admission
        })
    ));
    assert_eq!(
        gate.pending(),
        0,
        "deadline releases capacity without waiting for guard drop"
    );
    assert!(stream.write_all(&[0, 0, 0, 1, 5]).await.is_err());
    drop(guard);
    assert_eq!(gate.pending(), 0);
    assert_eq!(gate.stats().timed_out, 1);
}

#[tokio::test]
async fn wrong_expected_key_and_inner_protocol_are_refusals_without_fallback() {
    let gate = SetupGate::new(2);
    let wrong_key = ServiceSecurityPolicy::noise(
        protocol(),
        Some(PeerID::from_public_key(keys(99).public_key())),
        SetupLimits::default(),
    );
    let server_policy = policy();
    let (left, right) = tokio::io::duplex(8192);
    let (left, right) = tokio::join!(
        gate.noise_initiator(client_key(), left, &wrong_key),
        gate.noise_responder(server_key(), right, &server_policy)
    );
    assert!(matches!(left, Err(SetupError::PeerOrProtocolMismatch)));
    drop(right);
    let wrong_protocol =
        ServiceSecurityPolicy::noise(Alpn::new("mere/other/v1"), None, SetupLimits::default());
    let (left, right) = tokio::io::duplex(8192);
    let (left, right) = tokio::join!(
        gate.noise_initiator(client_key(), left, &server_policy),
        gate.noise_responder(server_key(), right, &wrong_protocol)
    );
    assert!(matches!(right, Err(SetupError::PeerOrProtocolMismatch)));
    drop(left);
    assert_eq!(gate.pending(), 0);
    assert_eq!(gate.stats().refused, 2);
}

#[tokio::test]
async fn plaintext_requires_its_explicit_owner_mode_and_never_claims_a_secure_peer() {
    let gate = SetupGate::new(1);
    let (stream, mut peer) = tokio::io::duplex(1024);
    assert!(matches!(
        gate.reserve(&policy()).unwrap().public_plaintext(stream),
        Err(SetupError::ModeMismatch)
    ));
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    let public = ServiceSecurityPolicy::public_plaintext(protocol(), SetupLimits::default());
    let (stream, _peer) = tokio::io::duplex(1024);
    let (_, facts, guard) = gate
        .reserve(&public)
        .unwrap()
        .public_plaintext(stream)
        .unwrap();
    assert_eq!(facts.peer(), None);
    assert_eq!(gate.stats().secured, 0);
    #[cfg(feature = "notochord")]
    assert!(facts.secure_channel_facts().is_none());
    guard.refused();
    assert_eq!(gate.pending(), 0);
}

#[tokio::test]
async fn malformed_oversized_and_truncated_frames_release_capacity_before_body_allocation() {
    for frame in [vec![0, 129], vec![0, 0], vec![0]] {
        let gate = SetupGate::new(1);
        let limits = SetupLimits {
            max_frame_bytes: 128,
            ..SetupLimits::default()
        };
        let policy = ServiceSecurityPolicy::noise(protocol(), None, limits);
        let (mut raw, stream) = tokio::io::duplex(1024);
        raw.write_all(&frame).await.unwrap();
        drop(raw);
        let result = gate.noise_responder(server_key(), stream, &policy).await;
        assert!(matches!(result, Err(SetupError::Security)));
        assert_eq!(gate.pending(), 0);
        assert_eq!(gate.stats().refused, 1);
    }
}

#[tokio::test]
async fn cumulative_byte_frame_and_work_budgets_are_enforced() {
    for limits in [
        SetupLimits {
            max_read_bytes: 33,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_write_bytes: 33,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_frames: 1,
            ..SetupLimits::default()
        },
        SetupLimits {
            max_io_operations: 1,
            ..SetupLimits::default()
        },
    ] {
        let gate = SetupGate::new(2);
        let policy = ServiceSecurityPolicy::noise(protocol(), None, limits);
        let (left, right) = tokio::io::duplex(8192);
        let (left, right) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                gate.noise_initiator(client_key(), left, &policy),
                gate.noise_responder(server_key(), right, &policy)
            )
        })
        .await
        .unwrap();
        assert!(left.is_err() && right.is_err());
        assert_eq!(gate.pending(), 0);
    }
}

#[tokio::test]
async fn bounded_wire_still_interoperates_with_the_legacy_noise_profile() {
    let gate = SetupGate::new(1);
    let policy = policy();
    let (left, right) = tokio::io::duplex(8192);
    let (left, right) = tokio::join!(
        gate.noise_initiator(client_key(), left, &policy),
        crate::noise::secure_responder(server_key(), right)
    );
    let (_, facts, guard) = left.unwrap();
    assert_eq!(
        facts.peer(),
        Some(PeerID::from_public_key(server_key().public_key()))
    );
    assert_eq!(
        right.unwrap().1,
        PeerID::from_public_key(client_key().public_key())
    );
    guard.admitted().unwrap();
    assert_eq!(gate.pending(), 0);
}
