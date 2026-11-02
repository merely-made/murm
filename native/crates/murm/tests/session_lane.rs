// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Notochord N2, Murm half: the owner's rule decides a real Murm connection
//! before the engine sees a byte.
//!
//! This is the low-power plan's V6 done-condition made concrete. The lane
//! carries the same posts the gossip lane does; what is new is that a peer has
//! to get past the owner's policy first, and a refused one leaves no trace in
//! the conversation. Every session here sets up through the lane's shared
//! security gate and secure v2 admission (consolidation T6): Noise over the
//! memory carrier, native iroh security over real p2panda.

#![cfg(feature = "session-lane")]

use std::collections::BTreeMap;
use std::sync::Arc;

use identity::delegation::Issue;
use identity::{Ed25519Keypair, IdentityProvider, InMemoryProvider};
use insigne::delegation::{
    CapabilityScope, DelegationCertificate, DelegationParent, SignedDelegationCertificate,
};
use murm::{
    Admission, CabalKey, ConversationEngine, Credentials, MurmError, Post, SessionLane,
    SessionOutcome,
};
use notochord::{
    DenyReason, LocalNetworkPolicy, NetworkId, ProfileRef, RequestedAction, RevocationLedger,
    SecureIdentityPolicy, ServiceAccess, ServiceRule, TrafficClass, TrustedRoot,
};
use transport::memory::MemoryTransport;
use transport::security::{SetupLimits, SetupStats};
use transport::{Alpn, PeerID, Transport};

const NETWORK: NetworkId = NetworkId([3; 32]);
const ROOT_AUTHORITY: [u8; 32] = [7; 32];
const MURM_SERVICE: &str = "/services/murm";
const PROTOCOL: &str = "mere/murm/v1";
const NOW_MS: u64 = 50;

fn root() -> InMemoryProvider {
    InMemoryProvider::from_seed([1; 32])
}

fn member() -> InMemoryProvider {
    InMemoryProvider::from_seed([4; 32])
}

fn member_key() -> [u8; 32] {
    member().master_public_key().to_bytes()
}

/// A peer holding no grant, whose own key is all it can prove.
fn stranger() -> InMemoryProvider {
    InMemoryProvider::from_seed([6; 32])
}

fn server_key() -> Ed25519Keypair {
    Ed25519Keypair::from_seed([8; 32])
}

fn peer_of(key: &Ed25519Keypair) -> PeerID {
    PeerID::from_public_key(key.public_key())
}

fn alpn() -> Alpn {
    Alpn::new(PROTOCOL)
}

fn lane() -> SessionLane {
    SessionLane::new(alpn(), 4, SetupLimits::default())
}

/// A grant for some other service's action triple.
///
/// Mirrors Graphshell's (`mere.graphshell` / `/services/projection`), spelled
/// out rather than imported: murm must not depend on a port, and the point of
/// the test is that a perfectly valid grant for *someone else's* service is
/// worthless here.
fn foreign_grant() -> SignedDelegationCertificate {
    SignedDelegationCertificate::issue(
        &root(),
        DelegationCertificate::new(
            DelegationParent::Root(ROOT_AUTHORITY),
            root().master_public_key().to_bytes(),
            member_key(),
            CapabilityScope {
                domain: "mere.graphshell".into(),
                resource: NETWORK.0.to_vec(),
                path_prefix: "/services/projection".into(),
                actions: ["connect".to_string()].into_iter().collect(),
            },
            5,
            10,
            Some(100),
            1,
            [2; 32],
        ),
    )
    .expect("issue certificate")
}

fn grant() -> SignedDelegationCertificate {
    SignedDelegationCertificate::issue(
        &root(),
        DelegationCertificate::new(
            DelegationParent::Root(ROOT_AUTHORITY),
            root().master_public_key().to_bytes(),
            member_key(),
            CapabilityScope {
                domain: "mere.network".into(),
                resource: NETWORK.0.to_vec(),
                path_prefix: MURM_SERVICE.into(),
                actions: ["connect".to_string()].into_iter().collect(),
            },
            5,
            10,
            Some(100),
            1,
            [1; 32],
        ),
    )
    .expect("issue certificate")
}

fn policy(access: ServiceAccess) -> LocalNetworkPolicy {
    let mut policy = LocalNetworkPolicy::closed(NETWORK);
    policy.accepted_profiles = vec![ProfileRef {
        id: "mere.base".into(),
        revision: 1,
    }];
    policy.trusted_roots = vec![TrustedRoot {
        authority: ROOT_AUTHORITY,
        issuer: root().master_public_key().to_bytes(),
    }];
    policy.services = BTreeMap::from([(
        MURM_SERVICE.to_string(),
        ServiceRule::new(access, "mere.network", ["connect"], false, None),
    )]);
    policy
}

fn credentials<'a>(
    subject: &'a InMemoryProvider,
    action: &str,
    delegations: Vec<SignedDelegationCertificate>,
) -> Credentials<'a, InMemoryProvider> {
    Credentials {
        identity: subject,
        network: NETWORK,
        profile: ProfileRef {
            id: "mere.base".into(),
            revision: 2,
        },
        action: RequestedAction {
            domain: "mere.network".into(),
            path: MURM_SERVICE.into(),
            action: action.into(),
        },
        class: TrafficClass::Interactive,
        nonce: [42; 32],
        delegations,
    }
}

/// A server engine holding one cabal, and a properly signed post for it.
///
/// The post is authored in the *sender's* engine so it carries a real
/// signature and the self-describing cabal id; the server ingests the encoded
/// form, exactly as it would off the gossip lane.
async fn fixtures() -> (Arc<ConversationEngine>, [u8; 32], Post) {
    let key = CabalKey::new([9u8; 32]);

    let sender = Arc::new(ConversationEngine::new(Arc::new(member())));
    let cabal_id = sender.open(*key.as_bytes()).await.expect("sender opens");
    let post_id = sender
        .post_text(&cabal_id, "general", "over the mesh", 1_000)
        .await
        .expect("author a post");
    let post = sender
        .get_post(&cabal_id, &post_id)
        .expect("the authored post");

    let server = Arc::new(ConversationEngine::new(Arc::new(
        InMemoryProvider::from_seed([8; 32]),
    )));
    server.open(*key.as_bytes()).await.expect("server opens");
    (server, cabal_id, post)
}

type Pushed = Result<Result<usize, DenyReason>, MurmError>;
type Served = Result<Result<SessionOutcome, DenyReason>, MurmError>;

/// One memory-carrier session through Noise, as the member would run it unless
/// a test changes one thing.
struct Exchange {
    access: ServiceAccess,
    /// The initiator's Noise key, which is also its memory carrier peer.
    key: Ed25519Keypair,
    /// The responder key the initiator's invitation names.
    expected: PeerID,
    action: &'static str,
    delegations: Vec<SignedDelegationCertificate>,
}

impl Exchange {
    fn member() -> Self {
        Self {
            access: ServiceAccess::MemberOnly,
            key: member().master_keypair().clone(),
            expected: peer_of(&server_key()),
            action: "connect",
            delegations: vec![grant()],
        }
    }
}

/// Both ends' results, how many posts the conversation holds afterwards, and
/// what the responder's gate counted.
struct Ran {
    pushed: Pushed,
    served: Served,
    held: usize,
    stats: SetupStats,
}

async fn run(exchange: Exchange) -> Ran {
    let (engine, cabal_id, post) = fixtures().await;
    let (client, server) = MemoryTransport::pair(peer_of(&exchange.key), peer_of(&server_key()));
    let server_lane = lane();

    let responder_lane = server_lane.clone();
    let responder_engine = Arc::clone(&engine);
    let owner = policy(exchange.access);
    let responder = tokio::spawn(async move {
        let accepted = server.accept(alpn()).await.expect("memory accept");
        responder_lane
            .serve_noise(
                &server_key(),
                accepted,
                &responder_engine,
                cabal_id,
                Admission {
                    policy: &owner,
                    ledger: &RevocationLedger::new(),
                    identity: SecureIdentityPolicy::MatchCarrierWhenPresent,
                    now_ms: NOW_MS,
                    active_sessions: 0,
                },
            )
            .await
    });

    let subject = member();
    let pushed = lane()
        .push_noise(
            &client,
            peer_of(&server_key()),
            &exchange.key,
            exchange.expected,
            credentials(&subject, exchange.action, exchange.delegations),
            std::slice::from_ref(&post),
        )
        .await;
    let served = responder.await.expect("responder task");
    Ran {
        pushed,
        served,
        held: engine.history(&cabal_id, "general").len(),
        stats: server_lane.stats(),
    }
}

fn refusals(ran: Ran) -> (DenyReason, DenyReason, usize) {
    let pushed = ran.pushed.expect("initiator setup").expect_err("refused");
    let served = ran.served.expect("responder setup").expect_err("refused");
    (pushed, served, ran.held)
}

#[tokio::test]
async fn an_admitted_peer_reaches_the_conversation() {
    let ran = run(Exchange::member()).await;
    assert_eq!(ran.pushed.expect("initiator setup"), Ok(1));
    let outcome = ran
        .served
        .expect("responder setup")
        .expect("a member with a valid grant is admitted");
    assert_eq!(outcome.posts_ingested, 1);
    assert_eq!(outcome.posts_rejected, 0);
    assert_eq!(
        outcome.principal.subject,
        member_key(),
        "traffic is attributed to the subject the proof established"
    );
    assert_eq!(ran.held, 1, "the post landed in the cabal");
    assert_eq!(ran.stats.admitted, 1);
    assert_eq!(ran.stats.pending, 0, "admission returned the setup lease");
}

#[tokio::test]
async fn a_refused_peer_never_reaches_the_engine() {
    // The V6 done-condition: the owner has not offered this service, so the
    // connection is refused before Murm receives application bytes.
    let ran = run(Exchange {
        access: ServiceAccess::Disabled,
        ..Exchange::member()
    })
    .await;
    let stats = ran.stats;
    let (pushed, served, held) = refusals(ran);
    assert_eq!(pushed, DenyReason::ServiceNotOffered);
    assert_eq!(served, DenyReason::ServiceNotOffered);
    assert_eq!(
        held, 0,
        "not one post crosses into the conversation on a refused session"
    );
    assert_eq!(stats.admission_refused, 1);
    assert_eq!(stats.pending, 0);
}

#[tokio::test]
async fn a_grant_for_another_service_does_not_open_murm() {
    // Half of the Notochord promotion gate: "a Graphshell projection grant
    // cannot open Murm". The chain is valid, signed by a root this node
    // trusts, and issued to the very peer that is connecting — it simply is
    // not a grant for *this* service, and that is the whole of the objection.
    let (pushed, served, held) = refusals(
        run(Exchange {
            delegations: vec![foreign_grant()],
            ..Exchange::member()
        })
        .await,
    );
    assert_eq!(pushed, DenyReason::ActionNotCovered);
    assert_eq!(
        served,
        DenyReason::ActionNotCovered,
        "a valid grant for another domain is still not authority here"
    );
    assert_eq!(held, 0, "and nothing it sent reaches the conversation");
}

#[tokio::test]
async fn an_unoffered_action_does_not_reach_murm() {
    let (_, served, held) = refusals(
        run(Exchange {
            action: "administer",
            ..Exchange::member()
        })
        .await,
    );
    assert_eq!(served, DenyReason::ActionNotOffered);
    assert_eq!(held, 0, "the service sees no bytes for an unoffered action");
}

#[tokio::test]
async fn a_channel_key_that_is_not_the_subject_is_refused() {
    // The member signs, but the channel belongs to another key. Under the
    // strict policy the owner chose, consent on a stranger's channel is not
    // the subject connecting.
    let (pushed, served, held) = refusals(
        run(Exchange {
            key: stranger().master_keypair().clone(),
            ..Exchange::member()
        })
        .await,
    );
    assert_eq!(pushed, DenyReason::SubjectNotSecurePeer);
    assert_eq!(served, DenyReason::SubjectNotSecurePeer);
    assert_eq!(held, 0);
}

#[tokio::test]
async fn a_server_the_invitation_did_not_name_gets_no_hello() {
    // The responder proves a key other than the invited one. The initiator
    // stops after Noise, before its hello and delegations leave, so the
    // responder is left waiting for a hello that never comes.
    let ran = run(Exchange {
        expected: peer_of(&Ed25519Keypair::from_seed([9; 32])),
        ..Exchange::member()
    })
    .await;
    assert!(
        ran.pushed.is_err(),
        "the initiator refuses the wrong server"
    );
    assert!(ran.served.is_err(), "the responder never reads a hello");
    assert_eq!(ran.held, 0);
    assert_eq!(ran.stats.admitted, 0);
}

mod p2panda {
    use super::*;
    use std::time::Duration;
    use transport::P2pandaTransport;

    static RECEIPT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn pair(
        endpoint: &InMemoryProvider,
    ) -> (P2pandaTransport, P2pandaTransport, PeerID, PeerID) {
        let client = P2pandaTransport::builder_from_seed_ref(&endpoint.master_keypair().to_seed())
            .alpns(vec![alpn()])
            .bind()
            .await
            .expect("bind Murm client");
        let server = P2pandaTransport::builder_from_seed_ref(&server_key().to_seed())
            .alpns(vec![alpn()])
            .bind()
            .await
            .expect("bind Murm server");
        let client_peer = client.local_peer_id();
        let server_peer = server.local_peer_id();

        let client_addr = tokio::time::timeout(Duration::from_secs(10), client.endpoint_addr())
            .await
            .expect("client endpoint address timeout")
            .expect("client endpoint address");
        let server_addr = tokio::time::timeout(Duration::from_secs(10), server.endpoint_addr())
            .await
            .expect("server endpoint address timeout")
            .expect("server endpoint address");
        client
            .add_peer(server_addr)
            .await
            .expect("client registers server");
        server
            .add_peer(client_addr)
            .await
            .expect("server registers client");

        (server, client, server_peer, client_peer)
    }

    /// The member pushes one post from `endpoint` to a server whose owner
    /// selected `identity`.
    async fn run_native(
        endpoint: &InMemoryProvider,
        identity: SecureIdentityPolicy,
    ) -> (Pushed, Served, usize) {
        let _receipt_guard = RECEIPT_LOCK.lock().await;
        let (engine, cabal_id, post) = fixtures().await;
        let (server, client, server_peer, client_peer) = pair(endpoint).await;

        let responder_engine = Arc::clone(&engine);
        let responder = tokio::spawn(async move {
            let accepted = tokio::time::timeout(Duration::from_secs(10), server.accept(alpn()))
                .await
                .expect("Murm accept timeout")
                .expect("Murm accept");
            assert_eq!(accepted.peer, Some(client_peer));
            lane()
                .serve_native(
                    accepted,
                    &responder_engine,
                    cabal_id,
                    Admission {
                        policy: &policy(ServiceAccess::MemberOnly),
                        ledger: &RevocationLedger::new(),
                        identity,
                        now_ms: NOW_MS,
                        active_sessions: 0,
                    },
                )
                .await
        });

        let subject = member();
        let pushed = tokio::time::timeout(
            Duration::from_secs(20),
            lane().push_native(
                &client,
                server_peer,
                credentials(&subject, "connect", vec![grant()]),
                std::slice::from_ref(&post),
            ),
        )
        .await
        .expect("Murm push timeout");
        let served = tokio::time::timeout(Duration::from_secs(20), responder)
            .await
            .expect("Murm responder timeout")
            .expect("Murm responder");
        (pushed, served, engine.history(&cabal_id, "general").len())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn authenticated_member_reaches_murm_over_real_p2panda() {
        let (pushed, served, held) =
            run_native(&member(), SecureIdentityPolicy::MatchCarrierWhenPresent).await;
        assert_eq!(pushed.expect("initiator setup"), Ok(1));
        let outcome = served
            .expect("responder setup")
            .expect("member admitted by responder");
        assert_eq!(outcome.posts_ingested, 1);
        assert_eq!(outcome.posts_rejected, 0);
        assert_eq!(outcome.principal.subject, member_key());
        assert_eq!(held, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_scoped_endpoint_key_is_admitted_by_bound_channel_proof() {
        // The endpoint key is not the member's root. The owner chose the
        // policy that accepts the root's signature over this exact stream.
        let (pushed, served, held) = run_native(
            &stranger(),
            SecureIdentityPolicy::BoundChannelApplicationProof,
        )
        .await;
        assert_eq!(pushed.expect("initiator setup"), Ok(1));
        let outcome = served
            .expect("responder setup")
            .expect("bound proof admits the member");
        assert_eq!(
            outcome.principal.subject,
            member_key(),
            "the principal is the signing root, not the endpoint key"
        );
        assert_eq!(held, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_scoped_endpoint_key_is_refused_under_the_strict_policy() {
        let (pushed, served, held) =
            run_native(&stranger(), SecureIdentityPolicy::MatchCarrierWhenPresent).await;
        assert_eq!(
            pushed.expect("initiator setup"),
            Err(DenyReason::SubjectNotSecurePeer)
        );
        assert_eq!(
            served.expect("responder setup").unwrap_err(),
            DenyReason::SubjectNotSecurePeer
        );
        assert_eq!(held, 0, "the refused connection leaves no post in Murm");
    }
}
