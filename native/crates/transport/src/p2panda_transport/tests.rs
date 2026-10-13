// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::TransportKind;
use identity::{IdentityProvider, InMemoryProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn make_inputs(seed: u8) -> (Ed25519Keypair, PeerID) {
    inputs_from([seed; 32])
}

fn inputs_from(seed: [u8; 32]) -> (Ed25519Keypair, PeerID) {
    let provider = InMemoryProvider::from_seed(seed);
    let kp = provider.master_keypair().clone();
    let peer_id = PeerID::from_public_key(provider.master_public_key());
    (kp, peer_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_seed_constructor_preserves_peer_identity() {
    let (keypair, expected_peer) = make_inputs(41);
    let transport = P2pandaTransport::bind_seed(keypair.to_seed(), vec![Alpn::new("mere/test/v1")])
        .await
        .expect("bind from external identity seed");

    assert_eq!(transport.local_peer_id(), expected_peer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paired_p2panda_transports_round_trip_bytes() {
    let alpn = Alpn::new("mere/test/v1");
    let (alice_kp, alice_id) = make_inputs(1);
    let (bob_kp, bob_id) = make_inputs(2);

    let alice = P2pandaTransport::bind(&alice_kp, vec![alpn.clone()])
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::bind(&bob_kp, vec![alpn.clone()])
        .await
        .expect("bind bob");

    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .expect("alice.add_peer");
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .expect("bob.add_peer");

    assert_eq!(alice.local_peer_id(), alice_id);
    assert_eq!(bob.local_peer_id(), bob_id);

    let payload = b"hello over p2panda-net".to_vec();
    let reply = b"hi back".to_vec();

    let alice_task = {
        let alpn = alpn.clone();
        let payload = payload.clone();
        let reply_len = reply.len();
        tokio::spawn(async move {
            let mut s = alice.connect(bob_id, alpn).await.expect("connect");
            s.write_all(&payload).await.expect("alice write");
            s.flush().await.expect("alice flush");
            let mut buf = vec![0u8; reply_len];
            s.read_exact(&mut buf).await.expect("alice read");
            buf
        })
    };

    let accepted = bob.accept(alpn.clone()).await.expect("bob accept");

    // p2panda authenticates its connections, so the accepted session reports
    // the real initiator (plan D4/V4) — not a claim read off the wire.
    assert_eq!(
        accepted.peer,
        Some(alice_id),
        "p2panda must report the authenticated initiator"
    );
    assert_ne!(
        accepted.peer,
        Some(bob_id),
        "the reported peer must be the initiator, not the acceptor"
    );
    assert!(accepted.is_transport_authenticated());
    assert_eq!(accepted.protocol, alpn);
    assert_eq!(accepted.ingress.transport, TransportKind::P2panda);

    let mut bob_stream = accepted.into_stream();
    let mut buf = vec![0u8; payload.len()];
    bob_stream.read_exact(&mut buf).await.expect("bob read");
    assert_eq!(buf, payload);
    bob_stream.write_all(&reply).await.expect("bob write");
    bob_stream.flush().await.expect("bob flush");

    let alice_recv = alice_task.await.expect("alice task");
    assert_eq!(alice_recv, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_keeps_a_small_final_frame_alive_until_acknowledged() {
    let alpn = Alpn::new("mere/final-frame/v1");
    let (alice_kp, alice_id) = make_inputs(11);
    let (bob_kp, bob_id) = make_inputs(12);
    let alice = P2pandaTransport::bind(&alice_kp, vec![alpn.clone()])
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::bind(&bob_kp, vec![alpn.clone()])
        .await
        .expect("bind bob");
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .expect("alice.add_peer");
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .expect("bob.add_peer");
    let (server_finished_tx, server_finished_rx) = tokio::sync::oneshot::channel();

    let alice_task = tokio::spawn(async move {
        let mut stream = alice.connect(bob_id, alpn).await.expect("connect");
        stream.write_all(b"request").await.expect("write request");
        stream.flush().await.expect("flush request");
        let mut final_frame = [0; 4];
        stream
            .read_exact(&mut final_frame)
            .await
            .expect("read final frame after responder shutdown");
        let _ = server_finished_rx.await;
        final_frame
    });

    let mut session = bob
        .accept(Alpn::new("mere/final-frame/v1"))
        .await
        .expect("accept");
    assert_eq!(session.peer, Some(alice_id));
    let mut request = [0; 7];
    session
        .stream
        .read_exact(&mut request)
        .await
        .expect("read request");
    assert_eq!(&request, b"request");
    session
        .stream
        .write_all(b"deny")
        .await
        .expect("write final frame");
    session.stream.shutdown().await.expect("finish final frame");
    let _ = server_finished_tx.send(());
    drop(session);

    let final_frame = alice_task.await.expect("alice task");
    assert_eq!(&final_frame, b"deny");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ticket_round_trips_identity_and_registers_the_peer() {
    let (alice_kp, alice_id) = make_inputs(30);
    let (bob_kp, _) = make_inputs(31);
    let alice = P2pandaTransport::bind(&alice_kp, vec![Alpn::new("mere/test/v1")])
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::bind(&bob_kp, vec![Alpn::new("mere/test/v1")])
        .await
        .expect("bind bob");

    // Alice shares a ticket string; bob parses it, which carries alice's
    // identity and registers her transport info for dialing.
    let ticket = alice.ticket().await.expect("alice ticket");
    assert!(
        !ticket.is_empty(),
        "the ticket serializes to a non-empty string"
    );
    let learned = bob
        .add_peer_ticket(&ticket)
        .await
        .expect("bob parses alice's ticket");
    assert_eq!(learned, alice_id, "the ticket round-trips alice's PeerID");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unregistered_alpn_accept_returns_error() {
    let (kp, _) = make_inputs(7);
    let t = P2pandaTransport::bind(&kp, vec![Alpn::new("mere/test/v1")])
        .await
        .expect("bind");
    let err = t
        .accept(Alpn::new("mere/other/v1"))
        .await
        .expect_err("must error");
    assert!(matches!(err, TransportError::AlpnNotRegistered));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binds_with_mdns_discovery() {
    // The discovery capability wires up: the mDNS service spawns off the
    // same endpoint without error. (End-to-end LAN discovery is multi-host
    // and not deterministically testable on a single machine.)
    let (kp, _) = make_inputs(9);
    let t = P2pandaTransport::builder(&kp)
        .alpns(vec![Alpn::new("mere/cable/v1")])
        .mdns(MdnsDiscoveryMode::Active)
        .bind()
        .await
        .expect("bind with mdns");
    assert!(t.endpoint_addr().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binds_with_random_walk_discovery() {
    // The random-walk discovery service spawns off the same endpoint and
    // its walker actors stay alive on the held handle. (End-to-end
    // discovery needs a populated bootstrap set + reachable peers, which is
    // a multi-host deployment concern, not single-machine-deterministic.)
    let (kp, _) = make_inputs(11);
    let t = P2pandaTransport::builder(&kp)
        .alpns(vec![Alpn::new("mere/cable/v1")])
        .discovery()
        .bind()
        .await
        .expect("bind with random-walk discovery");
    assert!(t.endpoint_addr().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binds_with_mdns_and_random_walk_together() {
    // LAN (mDNS) and internet (random-walk) discovery coexist on one
    // endpoint; both handles are held.
    let (kp, _) = make_inputs(12);
    let cfg = DiscoveryConfig {
        random_walkers_count: 4,
        ..DiscoveryConfig::default()
    };
    let t = P2pandaTransport::builder(&kp)
        .alpns(vec![Alpn::new("mere/cable/v1")])
        .mdns(MdnsDiscoveryMode::Active)
        .discovery_config(cfg)
        .bind()
        .await
        .expect("bind with mdns + random-walk");
    assert!(t.endpoint_addr().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gossip_propagates_ops_between_subscribed_peers() {
    use tokio_stream::StreamExt;

    let topic = [0x5e; 32];
    let (alice_kp, alice_id) = make_inputs(20);
    let (bob_kp, bob_id) = make_inputs(21);

    let alice = P2pandaTransport::builder(&alice_kp)
        .gossip()
        .bind()
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::builder(&bob_kp)
        .gossip()
        .bind()
        .await
        .expect("bind bob");

    // Explicit bootstrap: cross-register transport addresses and tag the
    // topic so gossip can form the overlay. (Discovery does this in
    // production; here we seed it deterministically.)
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    alice.set_topics(bob_id, &[topic]).await.unwrap();
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    bob.set_topics(alice_id, &[topic]).await.unwrap();

    // Both subscribe to the space topic.
    let alice_handle = alice.subscribe(topic).await.expect("alice subscribe");
    let bob_handle = bob.subscribe(topic).await.expect("bob subscribe");
    let mut bob_rx = bob_handle.subscribe();

    // Publish until bob receives (the gossip overlay needs a moment to form
    // the neighbour link), bounded by a timeout. This is a real convergence
    // proof: bytes Alice broadcasts reach Bob over the gossip overlay.
    let payload = b"a synced cabal operation".to_vec();
    let received = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            alice_handle
                .publish(payload.clone())
                .await
                .expect("publish");
            tokio::select! {
                msg = bob_rx.next() => {
                    if let Some(Ok(bytes)) = msg
                        && bytes == payload
                    {
                        break bytes;
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
    })
    .await
    .expect("bob received the gossip-broadcast operation within the timeout");

    assert_eq!(
        received, payload,
        "bob received exactly what alice published"
    );
}

/// A known address is not a live path, and the peer directory must say which
/// it has.
///
/// This is the distinction that hid a dead link for hours on 2026-08-03: a
/// firewall dropped every inbound packet to a device while its address stayed
/// in the address book, so the host reported the peer as reachable and looked
/// healthy while nothing replicated. `reachable` answers "do we know where it
/// lives"; `connected` answers "are we talking to it".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_peer_directory_separates_a_known_address_from_a_live_path() {
    use tokio_stream::StreamExt;

    let topic = [0x5b; 32];
    let (alice_kp, _) = make_inputs(90);
    let (bob_kp, bob_id) = make_inputs(91);

    let alice = P2pandaTransport::builder(&alice_kp)
        .gossip()
        .bind()
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::builder(&bob_kp)
        .gossip()
        .bind()
        .await
        .expect("bind bob");

    // Alice learns where Bob lives, and does not speak to him. This is the
    // state a paired-but-unreachable device sits in, and the state that used
    // to be indistinguishable from a working peer.
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    alice.set_topics(bob_id, &[topic]).await.unwrap();

    let known = alice
        .peers_for_topic(topic)
        .await
        .expect("directory")
        .into_iter()
        .find(|peer| peer.peer == bob_id)
        .expect("a peer with a registered address is in the directory");
    assert!(known.reachable, "his address is known");
    assert!(
        !known.connected,
        "knowing an address is not a live path: nothing has been sent yet"
    );

    // Now actually talk to him, over the gossip overlay rather than a
    // hand-rolled ALPN: a dial to a protocol the peer does not serve is
    // refused, which produces no path and would prove nothing. Bob does not
    // tag Alice, so only Alice dials: when both dial at once, iroh can mark a
    // live path inactive, and that case has its own test below.
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    // Bob subscribes first: gossip drops a join for a topic its receiver has
    // not subscribed to yet, and with one dialler nothing would retry it.
    let bob_handle = bob.subscribe(topic).await.expect("bob subscribe");
    let alice_handle = alice.subscribe(topic).await.expect("alice subscribe");
    let mut bob_rx = bob_handle.subscribe();
    let payload = b"traffic that forms a real path".to_vec();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            alice_handle
                .publish(payload.clone())
                .await
                .expect("publish");
            tokio::select! {
                msg = bob_rx.next() => {
                    if let Some(Ok(bytes)) = msg && bytes == payload { break }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
    })
    .await
    .expect("the overlay carried a message, so a path exists");

    let connected = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let directory = alice.peers_for_topic(topic).await.expect("directory");
            if let Some(peer) = directory.iter().find(|peer| peer.peer == bob_id)
                && peer.connected
            {
                break true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        connected,
        "a peer the endpoint holds an active path to must report connected"
    );

    // The cached-address rung rides on this: a connected peer's current
    // address set serializes to a ticket that round-trips back into the
    // address book and names the same peer. This is what a device persists
    // and reseeds after a restart.
    let ticket = alice
        .peer_ticket(bob_id)
        .await
        .expect("ticket query")
        .expect("a connected peer has addresses to serialize");
    let parsed = alice
        .add_peer_ticket(&ticket)
        .await
        .expect("the cached hint must round-trip through the ticket codec");
    assert_eq!(
        parsed, bob_id,
        "the ticket names the peer it was cached for"
    );

    // The readable route a directory reports: the live path is a direct
    // address (no relay is configured), marked active, and carried by the hint.
    let paths = alice.peer_paths(bob_id).await.expect("path query");
    let active: Vec<_> = paths.iter().filter(|path| path.active).collect();
    assert!(
        active
            .iter()
            .any(|path| matches!(path.addr, crate::PeerAddr::Direct(_))),
        "a connected peer reports its active direct path: {paths:?}"
    );
    let (named, carried) = crate::decode_peer_ticket(&ticket).expect("decode the hint");
    assert_eq!(named, bob_id, "the decoded hint names the same peer");
    for path in &active {
        assert!(
            carried.contains(&path.addr),
            "the live path {path:?} is one of the hint's addresses {carried:?}"
        );
    }
}

/// Pairing plan D1b: a peer tagged by id before any address is known is joined
/// once its address arrives. Before, the tag left it out of gossip's bootstrap
/// set and the late address never re-joined it, so neither side ever dialled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_tagged_before_its_address_is_joined_once_the_address_arrives() {
    use tokio_stream::StreamExt;

    let topic = [0x6c; 32];
    let (alice_kp, _) = make_inputs(120);
    let (bob_kp, bob_id) = make_inputs(121);
    let alice = P2pandaTransport::builder(&alice_kp)
        .gossip()
        .bind()
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::builder(&bob_kp)
        .gossip()
        .bind()
        .await
        .expect("bind bob");

    // Paired by id alone: listed at once, with no address and no path.
    alice.set_topics(bob_id, &[topic]).await.unwrap();
    let listed = alice
        .peers_for_topic(topic)
        .await
        .expect("directory")
        .into_iter()
        .find(|peer| peer.peer == bob_id)
        .expect("a peer tagged by id is listed before any address is known");
    assert!(!listed.reachable && !listed.connected, "{listed:?}");

    // Both join before Alice learns where Bob is; Bob knows nobody.
    let alice_handle = alice.subscribe(topic).await.expect("alice subscribe");
    let bob_handle = bob.subscribe(topic).await.expect("bob subscribe");
    let mut bob_rx = bob_handle.subscribe();
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();

    let payload = b"joined after the address arrived".to_vec();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            alice_handle
                .publish(payload.clone())
                .await
                .expect("publish");
            tokio::select! {
                msg = bob_rx.next() => {
                    if let Some(Ok(bytes)) = msg && bytes == payload { break }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
    })
    .await
    .expect("alice joined bob once his address arrived");
}

async fn within<T>(what: &str, limit: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, future)
        .await
        .unwrap_or_else(|_| panic!("{what} took over {limit:?}"))
}

/// Close an endpoint without failing on iroh's close: it has stalled past
/// 10 s after a both-sides dial, and no test here depends on it finishing.
async fn close_quietly(transport: &P2pandaTransport, who: &str) {
    if tokio::time::timeout(Duration::from_secs(10), transport.close())
        .await
        .is_err()
    {
        println!("{who}: close stalled past 10 s; dropping the endpoint");
    }
}

/// Two gossip transports that know each other's address, joined to `topic`.
/// With `both_dial`, each tags the other and both join at the same moment, so
/// each dials the other. Without it only Alice tags Bob, and Bob joins first:
/// gossip drops a join for a topic its receiver has not subscribed to yet.
async fn joined_pair(
    (alice_seed, bob_seed): ([u8; 32], [u8; 32]),
    topic: [u8; 32],
    both_dial: bool,
) -> (
    P2pandaTransport,
    GossipHandle,
    P2pandaTransport,
    GossipHandle,
) {
    let (alice_kp, alice_id) = inputs_from(alice_seed);
    let (bob_kp, bob_id) = inputs_from(bob_seed);
    let alice = P2pandaTransport::builder(&alice_kp).gossip().bind();
    let alice = within("bind", Duration::from_secs(10), alice)
        .await
        .unwrap();
    let bob = P2pandaTransport::builder(&bob_kp).gossip().bind();
    let bob = within("bind", Duration::from_secs(10), bob).await.unwrap();
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    alice.set_topics(bob_id, &[topic]).await.unwrap();
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    let (alice_handle, bob_handle) = within("subscribe", Duration::from_secs(10), async {
        if both_dial {
            bob.set_topics(alice_id, &[topic]).await.unwrap();
            tokio::join!(alice.subscribe(topic), bob.subscribe(topic))
        } else {
            let bob_handle = bob.subscribe(topic).await;
            (alice.subscribe(topic).await, bob_handle)
        }
    })
    .await;
    (alice, alice_handle.unwrap(), bob, bob_handle.unwrap())
}

/// Publishes a fresh payload every 100 ms one way and records when the other
/// side last received one. Fresh and tagged with its sender, because gossip
/// drops a payload it has seen, its own included.
struct Traffic {
    started: std::time::Instant,
    last: Arc<std::sync::atomic::AtomicU64>,
    tasks: [tokio::task::JoinHandle<()>; 2],
}

impl Traffic {
    fn start(sender: PeerID, from: &GossipHandle, to: &GossipHandle) -> Self {
        use std::sync::atomic::Ordering;
        use tokio_stream::StreamExt;
        let started = std::time::Instant::now();
        let last = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (seen, mut incoming) = (Arc::clone(&last), to.subscribe());
        let receive = tokio::spawn(async move {
            while let Some(message) = incoming.next().await {
                if message.is_ok() {
                    seen.store(started.elapsed().as_millis() as u64 + 1, Ordering::SeqCst);
                }
            }
        });
        let from = from.clone();
        let publish = tokio::spawn(async move {
            let mut sequence = 0u64;
            loop {
                sequence += 1;
                let payload = [sender.to_bytes().as_slice(), &sequence.to_le_bytes()].concat();
                let _ = from.publish(payload).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        Self {
            started,
            last,
            tasks: [receive, publish],
        }
    }

    /// A payload arrived within the last second.
    fn delivering(&self) -> bool {
        let last = self.last.load(std::sync::atomic::Ordering::SeqCst);
        last > 0 && (self.started.elapsed().as_millis() as u64) < last + 1000
    }
}

impl Drop for Traffic {
    fn drop(&mut self) {
        self.tasks.iter().for_each(|task| task.abort());
    }
}

/// Whether `me` reads `other` as connected, by the live rule or, with `live`
/// false, by iroh's path alone.
async fn reads_connected(
    me: &P2pandaTransport,
    other: PeerID,
    topic: [u8; 32],
    live: bool,
) -> bool {
    within(
        "directory",
        Duration::from_secs(5),
        me.peers_for_topic_by(topic, live),
    )
    .await
    .expect("directory")
    .iter()
    .any(|peer| peer.peer == other && peer.connected)
}

/// Both sides dialling at once must not read as a dead link while it delivers.
///
/// iroh then marks every address of the peer inactive for seconds at a time,
/// from its 5 s holepunch on or from the start, while gossip keeps carrying
/// traffic; that is why gossip decides on a subscribed overlay. The gap comes
/// in about a quarter of pairs in the parallel suite, so pairs are made until
/// the control, the same directory with iroh's path deciding, has read the
/// delivering link dead.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn both_sides_dialling_at_once_stays_connected_while_the_link_delivers() {
    // Pairs made before the control tripped, over 10 parallel runs of this
    // crate's suite (2026-10-03): 3, 2, 2, 1, 2, 9, 4, 8, 3, 3. An earlier
    // suite ran 16 pairs without a trip, so 48 is 3x that and 5x the worst
    // trip. Ten confirming runs at 48: 1, 12, 1, 8, 18, 4, 3, 2, 1, 1. Over
    // all 20, about 1 pair in 4.4 trips, so a miss is about 4 in 10^6
    // (ruling 37).
    const PAIRS: u16 = 48;
    const WINDOW: Duration = Duration::from_secs(7);
    // Per pair, so no key or topic is shared with another pair or test.
    let numbered = |tag: u8, pair: u16, side: u8| {
        let mut bytes = [tag; 32];
        bytes[..2].copy_from_slice(&pair.to_le_bytes());
        bytes[2] = side;
        bytes
    };
    let mut control = None;
    for pair in 0..PAIRS {
        let topic = numbered(0xd0, pair, 0);
        let seeds = (numbered(0xd1, pair, 0), numbered(0xd1, pair, 1));
        let (alice, alice_handle, bob, bob_handle) = joined_pair(seeds, topic, true).await;
        let (alice_id, bob_id) = (alice.local_peer_id(), bob.local_peer_id());
        let to_bob = Traffic::start(alice_id, &alice_handle, &bob_handle);
        let to_alice = Traffic::start(bob_id, &bob_handle, &alice_handle);
        let joined = std::time::Instant::now();
        while !(to_bob.delivering() && to_alice.delivering()) {
            assert!(
                joined.elapsed() < Duration::from_secs(20),
                "pair {pair}: no delivery both ways in 20 s (to bob {}, to alice {})",
                to_bob.delivering(),
                to_alice.delivering()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Sampled from the moment both ways deliver: iroh often marks the
        // path active only after the first delivery, and later drops it.
        let delivering = std::time::Instant::now();
        while delivering.elapsed() < WINDOW {
            let at = delivering.elapsed();
            assert!(
                to_bob.delivering() && to_alice.delivering(),
                "pair {pair}: the link stopped delivering at {at:?}"
            );
            for (me, other) in [(&alice, bob_id), (&bob, alice_id)] {
                assert!(
                    reads_connected(me, other, topic, true).await,
                    "pair {pair}: a delivering link read not connected at {at:?}"
                );
                if control.is_none() && !reads_connected(me, other, topic, false).await {
                    control = Some((pair, at));
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("pair {pair}: connected throughout {WINDOW:?}; control so far {control:?}");
        drop((to_bob, to_alice, alice_handle, bob_handle));
        close_quietly(&alice, "alice").await;
        close_quietly(&bob, "bob").await;
        if control.is_some() {
            break;
        }
    }
    let (pair, at) = control.unwrap_or_else(|| {
        panic!(
            "iroh's path never read a delivering link dead in {PAIRS} pairs: \
             this run proves nothing"
        )
    });
    println!("control: pair {pair}'s path read the delivering link dead {at:?} in");
}

/// A peer that closes reads not connected as soon as gossip reports it down,
/// though iroh keeps its path active for about a minute: on a subscribed
/// overlay the path does not decide. The control is that path, which must
/// still read the closed peer connected when the directory says it is not.
/// One-way dial, so the path is honestly active before the close.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_peer_reads_not_connected_once_its_neighbour_goes_down() {
    let topic = [0xe1; 32];
    let (alice, alice_handle, bob, bob_handle) =
        joined_pair(([240; 32], [241; 32]), topic, false).await;
    let bob_id = bob.local_peer_id();
    let to_bob = Traffic::start(alice.local_peer_id(), &alice_handle, &bob_handle);
    within(
        "connected by gossip and path",
        Duration::from_secs(20),
        async {
            while !(to_bob.delivering()
                && reads_connected(&alice, bob_id, topic, true).await
                && reads_connected(&alice, bob_id, topic, false).await)
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        },
    )
    .await;
    let listed = alice.peers_for_topic(topic).await.unwrap();
    assert!(
        listed.iter().all(|peer| peer.on_overlay),
        "subscribed, so gossip decides: {listed:?}"
    );

    drop((to_bob, bob_handle));
    let stopped = std::time::Instant::now();
    within("close", Duration::from_secs(10), bob.close())
        .await
        .unwrap();
    let closed = stopped.elapsed();
    drop(bob);
    let not_connected = within("not connected", Duration::from_secs(10), async {
        while reads_connected(&alice, bob_id, topic, true).await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        stopped.elapsed()
    })
    .await;
    let neighbour = alice
        .gossip_neighbours(topic)
        .await
        .contains(&bob_id.to_bytes());
    let path = reads_connected(&alice, bob_id, topic, false).await;
    println!(
        "stopped peer: close returned in {closed:?}; not connected {not_connected:?} \
         after close began; still a neighbour {neighbour}; path still active {path}"
    );
    assert!(
        !neighbour,
        "not connected because gossip reports the peer down"
    );
    assert!(
        not_connected < Duration::from_secs(2),
        "a closed peer reads not connected within a second or so: {not_connected:?}"
    );
    assert!(
        path,
        "the control: iroh still holds the closed peer's path active, so the \
         directory's answer came from gossip"
    );
    drop(alice_handle);
    close_quietly(&alice, "alice").await;
}

/// Holds an accepted connection open until the peer closes it.
#[derive(Debug, Clone)]
struct HoldOpen;

impl ProtocolHandler for HoldOpen {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let _ = connection.closed().await;
        Ok(())
    }
}

/// Connections the transport did not open are counted too: one through
/// `protocol_endpoint()`, as distillery's remote mounts its protocol, and one
/// gossip opened (rulings 47, 48).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connections_the_transport_did_not_open_are_counted() {
    let mounted = b"mere/test/mounted/v1";
    let (alice_kp, alice_id) = make_inputs(142);
    let (bob_kp, bob_id) = make_inputs(143);
    let alice = P2pandaTransport::bind(&alice_kp, Vec::new()).await.unwrap();
    let bob = P2pandaTransport::bind(&bob_kp, Vec::new()).await.unwrap();
    bob.protocol_endpoint()
        .accept(mounted, HoldOpen)
        .await
        .unwrap();
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    let (alice_key, bob_key) = (alice_id.to_bytes(), bob_id.to_bytes());
    assert_eq!(alice.open.count(&bob_key), 0, "nothing open yet");
    let conn = alice
        .protocol_endpoint()
        .connect(VerifyingKey::from_bytes(&bob_key).unwrap(), mounted)
        .await
        .unwrap();
    within("counted at both ends", Duration::from_secs(5), async {
        while !(alice.open.count(&bob_key) == 1 && bob.open.count(&alice_key) == 1) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    conn.close(0u32.into(), b"done");
    within("uncounted on close", Duration::from_secs(5), async {
        while alice.open.count(&bob_key) != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    close_quietly(&alice, "alice").await;
    close_quietly(&bob, "bob").await;

    // Gossip opens its own connections; Bob joins first and only Alice dials.
    let topic = [0x8e; 32];
    let (alice, alice_handle, bob, bob_handle) =
        joined_pair(([146; 32], [147; 32]), topic, false).await;
    let bob_key = bob.local_peer_id().to_bytes();
    let to_bob = Traffic::start(alice.local_peer_id(), &alice_handle, &bob_handle);
    within("gossip delivers", Duration::from_secs(20), async {
        while !to_bob.delivering() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        alice.open.count(&bob_key) >= 1,
        "gossip's connection is counted, and no other is open"
    );
    drop((to_bob, alice_handle, bob_handle));
    close_quietly(&alice, "alice").await;
    close_quietly(&bob, "bob").await;
}

/// Off the overlay, a peer that closes reads not connected within a second.
/// The control is iroh's path, still active then, so the count decided.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_off_the_overlay_that_closes_reads_not_connected_within_a_second() {
    let alpn = Alpn::new("mere/test/held/v1");
    let topic = [0x8d; 32];
    let (alice_kp, _) = make_inputs(148);
    let (bob_kp, bob_id) = make_inputs(149);
    let alice = P2pandaTransport::bind(&alice_kp, Vec::new()).await.unwrap();
    let bob = P2pandaTransport::bind(&bob_kp, vec![alpn.clone()])
        .await
        .unwrap();
    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .unwrap();
    alice.set_topics(bob_id, &[topic]).await.unwrap();
    let mut stream = alice.connect(bob_id, alpn).await.unwrap();
    stream.write_all(b"held").await.unwrap();
    stream.flush().await.unwrap();
    within("connected by both rules", Duration::from_secs(10), async {
        while !(reads_connected(&alice, bob_id, topic, true).await
            && reads_connected(&alice, bob_id, topic, false).await)
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    let listed = alice.peers_for_topic(topic).await.unwrap();
    assert!(
        listed.iter().all(|peer| !peer.on_overlay),
        "not subscribed, so the count decides: {listed:?}"
    );

    let closing = std::time::Instant::now();
    within("close", Duration::from_secs(10), bob.close())
        .await
        .unwrap();
    drop(bob);
    let not_connected = within("not connected", Duration::from_secs(5), async {
        while reads_connected(&alice, bob_id, topic, true).await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        closing.elapsed()
    })
    .await;
    let path = reads_connected(&alice, bob_id, topic, false).await;
    println!(
        "closed peer off the overlay: not connected {not_connected:?}; path still active {path}"
    );
    assert!(
        not_connected < Duration::from_secs(1),
        "a closed peer reads not connected within a second: {not_connected:?}"
    );
    assert!(
        path,
        "the control: iroh still lists the closed peer's path active, so the count decided"
    );
    drop(stream);
    close_quietly(&alice, "alice").await;
}

const KILLED_PEER_SEED: &str = "MERE_TRANSPORT_KILLED_PEER_SEED";
const KILLED_PEER_ALPN: &str = "mere/test/killed/v1";

/// The killed peer of the tests below, run by them as a child of this test
/// binary: binds, prints its ticket, and waits to be killed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a child process of the killed-peer tests"]
async fn killed_peer_child() {
    let Ok(seed) = std::env::var(KILLED_PEER_SEED) else {
        return;
    };
    let (keypair, _) = make_inputs(seed.parse().expect("seed"));
    let child = P2pandaTransport::bind(&keypair, vec![Alpn::new(KILLED_PEER_ALPN)])
        .await
        .expect("bind the child");
    // Serial libtest prints its label without a newline before this fixture.
    // Keep the strict ticket protocol on its own line in either harness mode.
    println!("\nCHILD_TICKET {}", child.ticket().await.expect("ticket"));
    use std::io::Write as _;
    std::io::stdout().flush().expect("flush");
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// Kills the child when dropped, so a failing test leaves nothing running.
struct ChildPeer(std::process::Child);

impl ChildPeer {
    async fn spawn(seed: u8) -> (Self, String) {
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "p2panda_transport::tests::killed_peer_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(KILLED_PEER_SEED, seed.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn the child");
        let stdout = child.stdout.take().expect("child stdout");
        let peer = ChildPeer(child);
        let reader = tokio::task::spawn_blocking(move || {
            use std::io::BufRead as _;
            std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .find_map(|line| line.strip_prefix("CHILD_TICKET ").map(str::to_string))
        });
        let ticket = within("child ticket", Duration::from_secs(60), reader)
            .await
            .expect("ticket reader")
            .expect("the child printed its ticket");
        (peer, ticket)
    }
}

impl Drop for ChildPeer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// When each rule let go of a killed child, measured from the kill.
#[derive(Debug)]
struct KilledPeer {
    /// The held connection's close event.
    closed: Duration,
    /// The live rule's first "not connected".
    not_connected: Duration,
    /// iroh's path rule, read once at 59 s. Not at 60 s: iroh drops the path
    /// 60 s after the peer's actor last stirred, which can be the kill itself.
    path_at_59_s: bool,
    /// iroh's path rule's first "not connected" from 59 s, if within 150 s.
    path_cleared: Option<Duration>,
}

/// A child off the overlay holding one own-ALPN connection, killed and read by
/// both rules, while `pollers` tasks read the directory and the child's paths
/// every 200 ms, as djinn's directory does.
async fn killed_peer_off_the_overlay(
    seed: u8,
    hooked: bool,
    pollers: usize,
) -> Result<KilledPeer, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let topic = [seed; 32];
    let (keypair, _) = make_inputs(seed);
    let mut builder = P2pandaTransport::builder(&keypair);
    if !hooked {
        builder = builder.without_connection_hook();
    }
    let parent = Arc::new(builder.bind().await.map_err(|e| e.to_string())?);
    let (mut child, ticket) = ChildPeer::spawn(seed + 1).await;
    let child_id = parent
        .add_peer_ticket(&ticket)
        .await
        .map_err(|e| e.to_string())?;
    parent
        .set_topics(child_id, &[topic])
        .await
        .map_err(|e| e.to_string())?;
    let connect = parent.connect(child_id, Alpn::new(KILLED_PEER_ALPN));
    let mut stream = within("connect", Duration::from_secs(20), connect)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(b"held").await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())?;
    let held = stream._connection.clone();
    let settling = std::time::Instant::now();
    loop {
        let live = reads_connected(&parent, child_id, topic, true).await;
        let path = reads_connected(&parent, child_id, topic, false).await;
        if live && path {
            break;
        }
        if settling.elapsed() > Duration::from_secs(10) {
            return Err(format!(
                "a live child off the overlay read not connected before the kill \
                 (live rule {live}, path rule {path})"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let polling: Vec<_> = (0..pollers)
        .map(|_| {
            let (parent, stop) = (Arc::clone(&parent), Arc::clone(&stop));
            tokio::spawn(async move {
                while !stop.load(Ordering::SeqCst) {
                    let _ = parent.peers_for_topic(topic).await;
                    let _ = parent.peer_paths(child_id).await;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
        })
        .collect();
    let killed = std::time::Instant::now();
    let close_event = tokio::spawn(async move {
        held.closed().await;
        killed.elapsed()
    });
    child.0.kill().map_err(|e| e.to_string())?;
    let _ = child.0.wait();
    let not_connected = loop {
        if !reads_connected(&parent, child_id, topic, true).await {
            break killed.elapsed();
        }
        if killed.elapsed() > Duration::from_secs(60) {
            return Err("the live rule still read the killed child connected at 60 s".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let closed = within("close event", Duration::from_secs(60), close_event)
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_secs(59).saturating_sub(killed.elapsed())).await;
    let path_at_59_s = reads_connected(&parent, child_id, topic, false).await;
    let mut path_cleared = None;
    while killed.elapsed() < Duration::from_secs(150) {
        if !reads_connected(&parent, child_id, topic, false).await {
            path_cleared = Some(killed.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    stop.store(true, Ordering::SeqCst);
    for task in polling {
        let _ = task.await;
    }
    drop(stream);
    close_quietly(&parent, "parent").await;
    Ok(KilledPeer {
        closed,
        not_connected,
        path_at_59_s,
        path_cleared,
    })
}

/// Off the overlay, a killed peer reads not connected when its last connection
/// closes, and three directory pollers do not move that delay; iroh's path
/// takes 60 s or more (rulings 47, 48, 53, 55).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_peer_off_the_overlay_reads_not_connected_when_its_connection_closes() {
    let (quiet, polled) = tokio::join!(
        killed_peer_off_the_overlay(130, true, 0),
        killed_peer_off_the_overlay(134, true, 3)
    );
    let (quiet, polled) = (quiet.unwrap(), polled.unwrap());
    for (name, run) in [("quiet", &quiet), ("three pollers", &polled)] {
        println!(
            "killed peer, {name}: close event {:?}, live rule not connected {:?}, \
             path rule at 59 s {}, path rule cleared {:?}",
            run.closed, run.not_connected, run.path_at_59_s, run.path_cleared
        );
    }
    for (name, run) in [("quiet", &quiet), ("polled", &polled)] {
        assert!(
            run.not_connected + Duration::from_millis(250) >= run.closed
                && run.not_connected <= run.closed + Duration::from_secs(2),
            "{name}: not connected within 2 s of the close event: {run:?}"
        );
        assert!(
            run.path_at_59_s
                && run
                    .path_cleared
                    .is_none_or(|cleared| cleared >= Duration::from_secs(60)),
            "{name}: the control, iroh's path, reads the killed peer connected for 60 s or \
             more: {run:?}"
        );
    }
    // Each run against its own close event (ruling 53): iroh's close itself
    // varies by seconds between runs, measured 9.98 s against 14.92 s.
    let delay = |run: &KilledPeer| run.not_connected.saturating_sub(run.closed);
    let moved = delay(&quiet).abs_diff(delay(&polled));
    assert!(
        moved <= Duration::from_secs(2),
        "polling moved the live rule's delay after the close event by {moved:?}"
    );
}

/// The killed-peer scenario fails without the hook: off the overlay, only the
/// count says a live peer is connected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_hook_the_killed_peer_scenario_fails() {
    let failed = killed_peer_off_the_overlay(138, false, 0)
        .await
        .expect_err("without the hook, the scenario must fail");
    assert!(failed.contains("before the kill"), "{failed}");
}
