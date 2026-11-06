// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Holders in process: acquire, retain, acknowledge, over memory and Noise/TCP.

use std::time::Duration;

use identity::Ed25519Keypair;
use muniment::MemoryBackend;
use murm_holder::{
    Ack, AckError, AckSink, CarrierClass, DialError, HolderError, HolderNode, HolderTarget,
    NoiseTcpDialer, Planned, Pool, PoolId, PoolRoute, PoolSetting, Reach, Replication,
    ReplicationStatus, Resolution, Retention, RoundError, TransportDialer, request_acks,
};
use murm_resolve::{Hint, Holder, NoiseTcpAdapter, Resolver, TransportAdapter, release_alpn};
use stickleback::edition::{
    ContentRef, EditionBody, EditionId, EditionKind, EditionOperation, EditionWriter, Provenance,
    SigningKey,
};
use transport::memory::MemoryTransport;
use transport::security::SetupLimits;
use transport::sync_lane::SyncLane;
use transport::{PeerID, Transport};

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn key(seed: u8) -> Ed25519Keypair {
    Ed25519Keypair::from_seed([seed; 32])
}

fn id_of(key: &Ed25519Keypair) -> PeerID {
    PeerID::from_public_key(key.public_key())
}

fn body(seed: u8) -> EditionBody {
    EditionBody {
        kind: EditionKind::Site,
        content: ContentRef::Snapshot {
            digest: EditionId([seed; 32]),
        },
        provenance: Provenance {
            source: "holder-test".into(),
            created_ms: u64::from(seed),
        },
        fork_of: None,
        hints: vec![],
        sealed: None,
    }
}

/// A publisher's log and the means to extend it.
struct Publisher {
    signer: SigningKey,
    writer: EditionWriter,
    editions: Vec<EditionOperation>,
    store: Holder<MemoryBackend>,
}

impl Publisher {
    async fn new(editions: u8) -> Self {
        let signer = SigningKey::generate();
        let mut me = Self {
            writer: EditionWriter::new(signer.clone()),
            signer,
            editions: vec![],
            store: Holder::new(MemoryBackend::new()),
        };
        for seed in 1..=editions {
            me.publish(seed).await;
        }
        me
    }

    async fn publish(&mut self, seed: u8) {
        let edition = self.writer.append(&body(seed)).unwrap();
        self.store.hold(std::slice::from_ref(&edition)).await;
        self.editions.push(edition);
    }

    fn log(&self) -> [u8; 32] {
        *self.signer.verifying_key().as_bytes()
    }

    fn head(&self) -> ([u8; 32], u32) {
        let last = self.editions.last().unwrap();
        (*last.hash.as_bytes(), last.header.seq_num)
    }

    fn replication(&self, holders: Vec<[u8; 32]>) -> Replication {
        let (head, seq) = self.head();
        Replication::new(self.log(), head, seq, holders)
    }
}

fn lane() -> SyncLane {
    SyncLane::new(release_alpn(), 4, SetupLimits::default())
}

fn forever() -> Retention {
    Retention {
        max_editions: 16,
        hold_until_ms: u64::MAX,
    }
}

fn node(seed: u8) -> HolderNode<MemoryBackend> {
    HolderNode::new(MemoryBackend::new(), key(seed), 8)
}

/// Serve one memory session from a publisher's store.
async fn publisher_serves(
    publisher: &Publisher,
    transport: &MemoryTransport,
    noise: Ed25519Keypair,
) {
    let served = async {
        let accepted = transport.accept(release_alpn()).await.unwrap();
        lane()
            .serve_noise(
                &noise,
                accepted.into_stream(),
                |_| true,
                publisher.store.session(),
            )
            .await
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), served).await;
}

/// Serve one memory session from a holder node.
async fn holder_serves(
    node: &HolderNode<MemoryBackend>,
    transport: &MemoryTransport,
    noise: Ed25519Keypair,
) {
    let served = async {
        let accepted = transport.accept(release_alpn()).await.unwrap();
        lane()
            .serve_noise(&noise, accepted.into_stream(), |_| true, node.service())
            .await
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), served).await;
}

/// The holder pulls from the publisher over memory.
async fn pull_memory(
    node: &HolderNode<MemoryBackend>,
    publisher: &Publisher,
) -> Result<murm_holder::Acquired, HolderError> {
    let (holder_t, publisher_t) = MemoryTransport::pair(id_of(&key(101)), id_of(&key(102)));
    let resolver = Resolver::new().with(
        TransportAdapter::new("memory", holder_t, key(103))
            .with_peers(vec![publisher_t.local_peer_id()]),
    );
    let (acquired, ()) = tokio::join!(
        node.acquire(&resolver, publisher.log(), &[]),
        publisher_serves(publisher, &publisher_t, key(104))
    );
    acquired
}

/// The publisher asks the holder for an acknowledgement over memory.
async fn ask_memory(
    node: &HolderNode<MemoryBackend>,
    replication: &mut Replication,
    holder_seed: u8,
) -> Vec<murm_holder::Round> {
    let (publisher_t, holder_t) = MemoryTransport::pair(id_of(&key(111)), id_of(&key(112)));
    let target = HolderTarget {
        holder: id_of(&key(holder_seed)),
        reach: Reach::Peer(holder_t.local_peer_id()),
    };
    let targets = [target];
    let dialer = TransportDialer::new("memory", publisher_t, key(113));
    let dialers: [&dyn murm_holder::Dial; 1] = [&dialer];
    let (rounds, ()) = tokio::join!(
        request_acks(replication, &targets, &dialers),
        holder_serves(node, &holder_t, key(holder_seed))
    );
    rounds
}

#[tokio::test]
async fn memory_holder_acquires_and_publishing_is_pending_until_it_acknowledges() {
    let mut publisher = Publisher::new(3).await;
    let holder = node(2);
    holder.agree(publisher.log(), forever()).unwrap();
    let holder_key = id_of(&key(2)).to_bytes();
    let mut replication = publisher.replication(vec![holder_key]);
    assert!(matches!(
        replication.status(),
        ReplicationStatus::Pending { .. }
    ));

    // The holder holds nothing yet: it cannot acknowledge.
    let rounds = ask_memory(&holder, &mut replication, 2).await;
    assert!(matches!(
        rounds[0].result,
        Err(RoundError::Dial(DialError::NotHeld))
    ));
    assert!(matches!(
        replication.status(),
        ReplicationStatus::Pending { .. }
    ));

    let acquired = pull_memory(&holder, &publisher).await.unwrap();
    assert_eq!(
        (acquired.before, acquired.after, acquired.added),
        (None, 2, 3)
    );
    let rounds = ask_memory(&holder, &mut replication, 2).await;
    assert!(rounds[0].result.is_ok(), "{rounds:?}");
    assert_eq!(replication.status(), ReplicationStatus::Replicated);

    // A new edition resets the acknowledgement until the holder catches up.
    publisher.publish(4).await;
    let (head, seq) = publisher.head();
    replication.advance(head, seq);
    assert!(matches!(
        replication.status(),
        ReplicationStatus::Pending { .. }
    ));
    let stale = ask_memory(&holder, &mut replication, 2).await;
    assert!(stale[0].result.is_ok());
    assert!(matches!(
        replication.status(),
        ReplicationStatus::Pending { .. }
    ));
    let acquired = pull_memory(&holder, &publisher).await.unwrap();
    assert_eq!(
        (acquired.before, acquired.after, acquired.added),
        (Some(2), 3, 1)
    );
    ask_memory(&holder, &mut replication, 2).await;
    assert_eq!(replication.status(), ReplicationStatus::Replicated);
}

#[tokio::test]
async fn holder_serves_what_it_holds_and_only_what_it_agreed_to() {
    let publisher = Publisher::new(2).await;
    let holder = node(2);
    holder.agree(publisher.log(), forever()).unwrap();
    pull_memory(&holder, &publisher).await.unwrap();

    let read = |log: [u8; 32]| {
        let holder = &holder;
        async move {
            let (reader_t, holder_t) = MemoryTransport::pair(id_of(&key(121)), id_of(&key(122)));
            let resolver = Resolver::new().with(
                TransportAdapter::new("memory", reader_t, key(123))
                    .with_peers(vec![holder_t.local_peer_id()]),
            );
            let address = format!("knot://{}/index.gmi", hex(&log)).parse().unwrap();
            let (read, ()) = tokio::join!(
                resolver.resolve(&address, &[]),
                holder_serves(holder, &holder_t, key(2))
            );
            read
        }
    };
    assert_eq!(
        read(publisher.log()).await.unwrap().chain.editions().len(),
        2
    );

    // Lapse the agreement: the data goes and the holder says not held.
    assert_eq!(holder.sweep(u64::MAX).await, vec![publisher.log()]);
    assert!(read(publisher.log()).await.is_err());
    assert_eq!(holder.head(&publisher.log()).await.unwrap(), None);
}

#[tokio::test]
async fn retention_bounds_logs_and_editions() {
    let publisher = Publisher::new(3).await;
    let holder = HolderNode::new(MemoryBackend::new(), key(2), 1);
    assert!(matches!(
        pull_memory(&holder, &publisher).await,
        Err(HolderError::NotAgreed)
    ));
    holder
        .agree(
            publisher.log(),
            Retention {
                max_editions: 2,
                hold_until_ms: 10,
            },
        )
        .unwrap();
    assert!(matches!(
        pull_memory(&holder, &publisher).await,
        Err(HolderError::OverBound { have: 3, max: 2 })
    ));
    assert_eq!(holder.head(&publisher.log()).await.unwrap(), None);
    assert!(matches!(
        holder.agree([9; 32], forever()),
        Err(HolderError::TooManyLogs(1))
    ));
    assert!(holder.sweep(9).await.is_empty());
    assert_eq!(holder.sweep(10).await.len(), 1);
}

#[tokio::test]
async fn a_conflicting_log_for_the_same_key_is_refused_and_a_shorter_one_adds_nothing() {
    let publisher = Publisher::new(3).await;
    let holder = node(2);
    holder.agree(publisher.log(), forever()).unwrap();
    pull_memory(&holder, &publisher).await.unwrap();

    // The same key signing different editions at the same positions.
    let mut rival = Publisher {
        writer: EditionWriter::new(publisher.signer.clone()),
        signer: publisher.signer.clone(),
        editions: vec![],
        store: Holder::new(MemoryBackend::new()),
    };
    for seed in [41, 42] {
        rival.publish(seed).await;
    }
    assert!(matches!(
        pull_memory(&holder, &rival).await,
        Err(HolderError::Conflict(_))
    ));
    assert_eq!(holder.head(&publisher.log()).await.unwrap().unwrap().1, 2);

    // A prefix of what is held is valid and changes nothing.
    let mut prefix = Publisher {
        writer: EditionWriter::new(publisher.signer.clone()),
        signer: publisher.signer.clone(),
        editions: vec![],
        store: Holder::new(MemoryBackend::new()),
    };
    prefix.publish(1).await;
    let kept = pull_memory(&holder, &prefix).await.unwrap();
    assert_eq!((kept.before, kept.after, kept.added), (Some(2), 2, 0));
}

mod hostile {
    use murm_sync::{
        StreamSession, StreamSync, StreamSyncError, StreamSyncLimits, StreamSyncOutcome,
    };
    use stickleback::MunimentStore;
    use stickleback::edition::{EditionExt, edition_topic};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use super::*;

    /// Claims to hold the wanted log and sends another key's editions.
    pub struct Liar {
        pub store: MunimentStore<MemoryBackend, EditionExt>,
        pub wanted: [u8; 32],
    }

    impl StreamSession for Liar {
        async fn run<St>(self, mut stream: St) -> Result<StreamSyncOutcome, StreamSyncError>
        where
            St: AsyncRead + AsyncWrite + Unpin,
        {
            let mut request = [0u8; 34];
            stream.read_exact(&mut request).await.unwrap();
            stream.write_all(&[1]).await.unwrap();
            stream.flush().await.unwrap();
            StreamSync::<_, u64, EditionExt, _>::new(
                edition_topic(&self.wanted),
                self.store,
                StreamSyncLimits::default(),
                |_| std::future::ready(false),
            )
            .run(stream)
            .await
        }
    }
}

#[tokio::test]
async fn a_holder_refuses_editions_not_signed_by_the_log_key() {
    use stickleback::MunimentStore;
    use stickleback::edition::{EditionExt, edition_topic};

    let wanted = Publisher::new(2).await;
    let foreign = Publisher::new(2).await;
    let store = MunimentStore::<MemoryBackend, EditionExt>::new(MemoryBackend::new());
    for edition in &foreign.editions {
        store
            .insert_indexed_operation(&edition_topic(&wanted.log()), edition, &0u64)
            .await
            .unwrap();
    }
    let liar = hostile::Liar {
        store,
        wanted: wanted.log(),
    };
    let holder = node(2);
    holder.agree(wanted.log(), forever()).unwrap();
    let (holder_t, liar_t) = MemoryTransport::pair(id_of(&key(131)), id_of(&key(132)));
    let resolver = Resolver::new().with(
        TransportAdapter::new("memory", holder_t, key(133))
            .with_peers(vec![liar_t.local_peer_id()]),
    );
    let serve = async {
        let accepted = liar_t.accept(release_alpn()).await.unwrap();
        let _ = lane()
            .serve_noise(&key(134), accepted.into_stream(), |_| true, liar)
            .await;
    };
    let (acquired, ()) = tokio::join!(holder.acquire(&resolver, wanted.log(), &[]), serve);
    assert!(
        matches!(acquired, Err(HolderError::Resolve(_))),
        "{acquired:?}"
    );
    assert_eq!(holder.head(&wanted.log()).await.unwrap(), None);
}

#[test]
fn acknowledgements_bind_holder_log_head_and_challenge() {
    let (holder, log, head, challenge) = (key(2), [7; 32], [8; 32], [9; 32]);
    let ack = Ack::sign(&holder, log, head, 3, challenge);
    let who = holder.public_key().to_bytes();
    assert_eq!(ack.verify(&who, &log, &challenge), Ok(()));
    assert_eq!(Ack::decode(&ack.encode()).unwrap(), ack);
    assert_eq!(
        ack.verify(&who, &log, &[1; 32]),
        Err(AckError::WrongChallenge)
    );
    assert_eq!(
        ack.verify(&who, &[1; 32], &challenge),
        Err(AckError::WrongLog)
    );
    assert_eq!(
        ack.verify(&[1; 32], &log, &challenge),
        Err(AckError::WrongHolder)
    );
    // Changing the head or sequence after signing breaks the signature.
    let mut bytes = ack.encode();
    bytes[64] ^= 1;
    assert_eq!(
        Ack::decode(&bytes).unwrap().verify(&who, &log, &challenge),
        Err(AckError::BadSignature)
    );
    // A holder outside the configured set cannot acknowledge.
    let mut replication = Replication::new(log, head, 3, vec![[5; 32]]);
    assert_eq!(
        replication.accept(&ack, &challenge),
        Err(AckError::WrongHolder)
    );
    assert!(matches!(
        replication.status(),
        ReplicationStatus::Pending { .. }
    ));
    assert_eq!(
        Replication::new(log, head, 3, vec![]).status(),
        ReplicationStatus::Unconfigured
    );
}

#[tokio::test]
async fn noise_tcp_holder_acquires_and_acknowledges() {
    let publisher = Publisher::new(3).await;
    let holder = node(5);
    holder.agree(publisher.log(), forever()).unwrap();
    let (publisher_key, holder_key) = (key(4), key(5));

    let publisher_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let publisher_addr = publisher_listener.local_addr().unwrap();
    let serve_publisher = async {
        let Ok(Ok((tcp, _))) =
            tokio::time::timeout(Duration::from_secs(10), publisher_listener.accept()).await
        else {
            return;
        };
        let _ = lane()
            .serve_noise(&publisher_key, tcp, |_| true, publisher.store.session())
            .await;
    };
    let hint = Hint::parse(&format!(
        "noise+tcp://{publisher_addr}/{}",
        hex(&id_of(&key(4)).to_bytes())
    ))
    .unwrap();
    let hints = [hint];
    let resolver = Resolver::new().with(NoiseTcpAdapter::new(key(6)));
    let (acquired, ()) = tokio::join!(
        holder.acquire(&resolver, publisher.log(), &hints),
        serve_publisher
    );
    assert_eq!(acquired.unwrap().after, 2);

    let holder_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let holder_addr = holder_listener.local_addr().unwrap();
    let serve_holder = async {
        let Ok(Ok((tcp, _))) =
            tokio::time::timeout(Duration::from_secs(10), holder_listener.accept()).await
        else {
            return;
        };
        let _ = lane()
            .serve_noise(&holder_key, tcp, |_| true, holder.service())
            .await;
    };
    let mut replication = publisher.replication(vec![id_of(&key(5)).to_bytes()]);
    let target = HolderTarget {
        holder: id_of(&key(5)),
        reach: Reach::NoiseTcp(holder_addr),
    };
    let dialer = NoiseTcpDialer::new(key(7));
    let dialers: [&dyn murm_holder::Dial; 1] = [&dialer];
    let targets = [target];
    let (rounds, ()) = tokio::join!(
        request_acks(&mut replication, &targets, &dialers),
        serve_holder
    );
    assert!(rounds[0].result.is_ok(), "{rounds:?}");
    assert_eq!(replication.status(), ReplicationStatus::Replicated);

    // A holder that is not the configured one cannot answer for it: the
    // dialer requires the configured key, so Noise refuses.
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_addr = other.local_addr().unwrap();
    let impostor = node(8);
    impostor.agree(publisher.log(), forever()).unwrap();
    let serve_other = async {
        if let Ok(Ok((tcp, _))) =
            tokio::time::timeout(Duration::from_secs(10), other.accept()).await
        {
            let _ = lane()
                .serve_noise(&key(8), tcp, |_| true, impostor.service())
                .await;
        }
    };
    let mut fresh = publisher.replication(vec![id_of(&key(5)).to_bytes()]);
    let target = HolderTarget {
        holder: id_of(&key(5)),
        reach: Reach::NoiseTcp(other_addr),
    };
    let targets = [target];
    let (rounds, ()) = tokio::join!(request_acks(&mut fresh, &targets, &dialers), serve_other);
    assert!(rounds[0].result.is_err());
    assert!(matches!(fresh.status(), ReplicationStatus::Pending { .. }));
}

#[test]
fn the_pool_setting_shows_each_routes_tier_and_refuses_below_the_floor() {
    let target = |seed: u8| HolderTarget {
        holder: id_of(&key(seed)),
        reach: Reach::Peer(id_of(&key(seed))),
    };
    let mesh = PoolId::PersonalMesh;
    let named = PoolId::Named("friends".into());
    let setting = PoolSetting {
        pools: vec![
            Pool {
                id: mesh.clone(),
                routes: vec![PoolRoute {
                    target: target(1),
                    class: CarrierClass::Iroh,
                }],
            },
            Pool {
                id: named.clone(),
                routes: vec![
                    PoolRoute {
                        target: target(2),
                        class: CarrierClass::Reticulum,
                    },
                    PoolRoute {
                        target: target(3),
                        class: CarrierClass::Onion,
                    },
                ],
            },
        ],
        selected: vec![mesh.clone(), named.clone(), PoolId::Named("gone".into())],
        floor: Resolution::MeshPseudonym,
    };
    let plan = setting.plan(true);
    let kept: Vec<_> = plan
        .routes
        .iter()
        .map(|Planned { pool, tier, .. }| ((*pool).clone(), tier.resolution))
        .collect();
    assert_eq!(
        kept,
        vec![
            (named.clone(), Resolution::MeshPseudonym),
            (named, Resolution::Hidden)
        ]
    );
    assert_eq!(plan.below_floor.len(), 1);
    assert_eq!(plan.below_floor[0].pool, &mesh);
    assert_eq!(plan.unknown_pools.len(), 1);
    assert_eq!(plan.targets().len(), 2);
    assert!(
        plan.routes
            .iter()
            .all(|p| p.tier.data == murm_holder::DataVisible::Sealed)
    );
}
