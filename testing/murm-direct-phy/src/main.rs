//! V7: Murm owner-policy admission over Reticulum and two direct-PHY radios.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use identity::delegation::{
    CapabilityScope, DelegationCertificate, DelegationParent, SignedDelegationCertificate,
};
use identity::{Ed25519Keypair, IdentityProvider, InMemoryProvider};
use murm::{
    CabalKey, ConversationEngine, Post, SessionOutcome, push_posts, serve_accepted_session,
};
use notochord::{
    DenyReason, LocalNetworkPolicy, NetworkId, ProfileRef, RequestedAction, RevocationLedger,
    ServiceAccess, ServiceRule, SessionHello, TrafficClass, TrustedRoot,
};
use retinue::iface::tulle::drive;
use transport::{Alpn, PeerID, ReticulumTransport, Transport, initiator_link_binding};
use tulle::PhyProfile;
use tulle::airtime::AirtimeBudget;
use tulle::direct_phy_serial::{DirectPhySerialConfig, DirectPhySerialLink};

const NETWORK: NetworkId = NetworkId([3; 32]);
const ROOT_AUTHORITY: [u8; 32] = [7; 32];
const MURM_SERVICE: &str = "/services/murm";
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

fn profile(frequency_hz: u32, bandwidth_hz: u32) -> PhyProfile {
    PhyProfile {
        frequency_hz,
        bandwidth_hz,
        spreading_factor: 8,
        coding_rate_denominator: 5,
        preamble_symbols: 16,
        sync_word: 0x12,
        explicit_header: true,
        crc: true,
        invert_iq: false,
        tx_power_dbm: 17,
    }
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

fn hello(binding: &notochord::ProofBinding) -> SessionHello {
    SessionHello::issue(
        &member(),
        NETWORK,
        ProfileRef {
            id: "mere.base".into(),
            revision: 2,
        },
        RequestedAction {
            domain: "mere.network".into(),
            path: MURM_SERVICE.into(),
            action: "connect".into(),
        },
        TrafficClass::Interactive,
        [42; 32],
        binding,
        vec![grant()],
    )
    .expect("issue hello")
}

async fn fixtures() -> (Arc<ConversationEngine>, [u8; 32], Post) {
    let key = CabalKey::new([9u8; 32]);
    let sender = Arc::new(ConversationEngine::new(Arc::new(member())));
    let cabal_id = sender.open(*key.as_bytes()).await.expect("sender opens");
    let post_id = sender
        .post_text(&cabal_id, "general", "over direct PHY", 1_000)
        .await
        .expect("author post");
    let post = sender.get_post(&cabal_id, &post_id).expect("authored post");

    let server = Arc::new(ConversationEngine::new(Arc::new(
        InMemoryProvider::from_seed([8; 32]),
    )));
    server.open(*key.as_bytes()).await.expect("server opens");
    (server, cabal_id, post)
}

async fn accept_once(
    server: Arc<ReticulumTransport>,
    alpn: Alpn,
    engine: Arc<ConversationEngine>,
    cabal_id: [u8; 32],
    access: ServiceAccess,
) -> Result<SessionOutcome, DenyReason> {
    let accepted = tokio::time::timeout(Duration::from_secs(90), server.accept(alpn))
        .await
        .expect("RF accept timed out")
        .expect("RF accept failed");
    assert_eq!(accepted.peer, None);
    let facts = accepted.session_facts();
    assert!(
        facts.ingress.local_interface.is_some(),
        "the receiving radio interface must survive admission"
    );
    assert!(
        facts.ingress.shared_link.is_some(),
        "the Reticulum link must bind the session proof"
    );

    let owner_policy = policy(access);
    let ledger = RevocationLedger::new();
    tokio::time::timeout(
        Duration::from_secs(150),
        serve_accepted_session(
            accepted,
            &engine,
            cabal_id,
            &owner_policy,
            &ledger,
            NOW_MS,
            0,
        ),
    )
    .await
    .expect("Murm responder timed out")
    .expect("Murm responder failed")
}

async fn push_once(
    client: &ReticulumTransport,
    server_peer: PeerID,
    alpn: &Alpn,
    post: &Post,
) -> Result<usize, DenyReason> {
    let stream = tokio::time::timeout(
        Duration::from_secs(90),
        client.connect(server_peer, alpn.clone()),
    )
    .await
    .expect("RF connect timed out")
    .expect("RF connect failed");
    let binding = initiator_link_binding(alpn, stream.link_id());
    let owner_policy = policy(ServiceAccess::MemberOnly);
    tokio::time::timeout(
        Duration::from_secs(150),
        push_posts(
            stream,
            &hello(&binding),
            &owner_policy,
            std::slice::from_ref(post),
        ),
    )
    .await
    .expect("Murm initiator timed out")
    .expect("Murm initiator failed")
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let client_port = args.next().unwrap_or_else(|| "COM6".into());
    let server_port = args.next().unwrap_or_else(|| "COM10".into());
    let frequency_hz = args
        .next()
        .map(|value| value.parse::<u32>())
        .transpose()?
        .unwrap_or(906_875_000);
    let bandwidth_hz = args
        .next()
        .map(|value| value.parse::<u32>())
        .transpose()?
        .unwrap_or(250_000);

    let radio_config = DirectPhySerialConfig {
        online_timeout: Duration::from_secs(10),
        transmit_timeout: Duration::from_secs(10),
        ..DirectPhySerialConfig::default()
    };
    let mut client_radio = DirectPhySerialLink::open(
        &client_port,
        profile(frequency_hz, bandwidth_hz),
        AirtimeBudget::new(60_000, 60_000),
        radio_config.clone(),
    )?;
    let mut server_radio = DirectPhySerialLink::open(
        &server_port,
        profile(frequency_hz, bandwidth_hz),
        AirtimeBudget::new(60_000, 60_000),
        radio_config,
    )?;
    tokio::time::timeout(Duration::from_secs(15), client_radio.wait_online()).await??;
    tokio::time::timeout(Duration::from_secs(15), server_radio.wait_online()).await??;
    println!(
        "radios online: {client_port}=client, {server_port}=server, \
         {frequency_hz} Hz/{bandwidth_hz} Hz"
    );

    let alpn = Alpn::new("mere/murm/v1");
    let server_kp = Ed25519Keypair::from_seed([21u8; 32]);
    let client_kp = Ed25519Keypair::from_seed([22u8; 32]);
    let server_peer = PeerID::from_public_key(server_kp.public_key());

    let server = Arc::new(
        ReticulumTransport::builder(&server_kp)
            .alpns(vec![alpn.clone()])
            .announce_interval(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(90))
            .link_mtu(255)
            .reliable_links(true)
            .reliable_initial_rtt(Duration::from_secs(2))
            .reliable_max_window(1)
            .bind()
            .await?,
    );
    let client = Arc::new(
        ReticulumTransport::builder(&client_kp)
            .alpns(vec![alpn.clone()])
            .announce_interval(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(90))
            .link_mtu(255)
            .reliable_links(true)
            .reliable_initial_rtt(Duration::from_secs(2))
            .reliable_max_window(1)
            .bind()
            .await?,
    );

    let client_for_driver = Arc::clone(&client);
    let client_driver = tokio::spawn(async move {
        let result = drive(client_for_driver.attach_packet_interface(), client_radio).await;
        if let Err(error) = &result {
            eprintln!("client radio driver stopped: {error}");
        }
        result
    });
    let server_for_driver = Arc::clone(&server);
    let server_driver = tokio::spawn(async move {
        let result = drive(server_for_driver.attach_packet_interface(), server_radio).await;
        if let Err(error) = &result {
            eprintln!("server radio driver stopped: {error}");
        }
        result
    });
    server.announce_now();
    let (engine, cabal_id, post) = fixtures().await;

    let admitted = tokio::spawn(accept_once(
        Arc::clone(&server),
        alpn.clone(),
        Arc::clone(&engine),
        cabal_id,
        ServiceAccess::MemberOnly,
    ));
    let sent = push_once(&client, server_peer, &alpn, &post).await;
    let admitted = admitted.await.expect("admission task");
    assert_eq!(sent, Ok(1));
    let outcome = admitted.expect("valid member grant must admit");
    assert_eq!(outcome.posts_ingested, 1);
    assert_eq!(outcome.posts_rejected, 0);
    assert_eq!(engine.history(&cabal_id, "general").len(), 1);
    println!("admitted: one signed Murm post landed over direct PHY");

    tokio::time::sleep(Duration::from_secs(2)).await;

    let refused = tokio::spawn(accept_once(
        Arc::clone(&server),
        alpn.clone(),
        Arc::clone(&engine),
        cabal_id,
        ServiceAccess::Disabled,
    ));
    let sent = push_once(&client, server_peer, &alpn, &post).await;
    let refused = refused.await.expect("refusal task");
    assert_eq!(sent, Err(DenyReason::ServiceNotOffered));
    assert_eq!(refused, Err(DenyReason::ServiceNotOffered));
    assert_eq!(
        engine.history(&cabal_id, "general").len(),
        1,
        "a refused session must leave the conversation unchanged"
    );
    println!("refused: disabled owner rule stopped the post before Murm");

    tokio::join!(
        client.shutdown(Duration::from_secs(5)),
        server.shutdown(Duration::from_secs(5))
    );
    tokio::time::timeout(Duration::from_secs(10), client_driver)
        .await
        .expect("client radio driver did not stop")??;
    tokio::time::timeout(Duration::from_secs(10), server_driver)
        .await
        .expect("server radio driver did not stop")??;

    println!("MURM V7 DIRECT-PHY HEADED PASSED");
    Ok(())
}
