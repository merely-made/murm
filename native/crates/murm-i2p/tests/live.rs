//! I2P2's live receipt: two retinue endpoints on one embedded emissary exchange an announce,
//! a link and a Resource in both directions, after the same exchange over loopback TCP as
//! the positive control in the same run.
//!
//! Ignored by default: it reseeds from the public I2P network and builds real tunnels.
//!
//! ```sh
//! MURM_I2P_STORAGE=<dir> cargo test -p murm-i2p --test live -- --ignored --nocapture
//! ```
//!
//! `MURM_I2P_RECEIPT=<file>` also writes the measurements as JSON.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use murm_i2p::{EmbeddedRouter, RouterConfig, Transit};
use retinue::destination::DestinationName;
use retinue::endpoint::{
    Endpoint, HwMtu, I2pListen, I2pPeer, ListenPolicy, PayloadMode, ReceivedPayload,
    ResourceTransferConfig, TcpClient,
};
use retinue::hash::AddressHash;
use retinue::identity::{Identity, PrivateIdentity};

/// What one exchange measured, each from the start of the exchange.
#[derive(Debug)]
struct Exchange {
    announces: Duration,
    link_a_to_b: Duration,
    link_b_to_a: Duration,
    resource_a_to_b: Duration,
    resource_b_to_a: Duration,
}

/// An endpoint with an echo destination and a resource destination, serving both.
fn node(seed: u8, tag: &'static str) -> (Arc<Endpoint>, [DestinationName; 2]) {
    let ep = Arc::new(Endpoint::new(PrivateIdentity::from_secret_bytes(
        &[seed; 64],
    )));
    let echo = DestinationName::new("receipt", ["echo", tag]);
    let resource = DestinationName::new("receipt", ["resource", tag]);
    ep.register(echo.clone(), b"echo");
    ep.register_resource(resource.clone(), b"resource");
    let server = Arc::clone(&ep);
    tokio::spawn(async move {
        while let Ok(mut stream) = server.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut reply = b"echo:".to_vec();
                    reply.extend_from_slice(&buf[..n]);
                    if stream.write_all(&reply).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (ep, [echo, resource])
}

/// Announce `names` every two seconds until `listener` has heard both, returning what it
/// heard keyed by app data ("echo", "resource").
async fn hear(
    speaker: &Arc<Endpoint>,
    names: &[DestinationName; 2],
    listener: &Arc<Endpoint>,
) -> HashMap<Vec<u8>, (AddressHash, Identity)> {
    let mut heard = HashMap::new();
    while heard.len() < 2 {
        speaker.announce(&names[0], b"echo");
        speaker.announce(&names[1], b"resource");
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        while let Ok(Ok(fact)) = tokio::time::timeout_at(until, listener.next_announcement()).await
        {
            heard.insert(fact.app_data.clone(), (fact.destination, fact.identity));
        }
    }
    heard
}

async fn echo(from: &Endpoint, (dest, identity): (AddressHash, Identity), msg: &str) {
    let mut stream = from.open(dest, identity).await.expect("link opens");
    stream.write_all(msg.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], format!("echo:{msg}").as_bytes());
}

async fn resource(from: &Endpoint, to: &Arc<Endpoint>, (dest, identity): (AddressHash, Identity)) {
    let payload: Vec<u8> = (0..12_000_u32)
        .map(|n| n.wrapping_mul(31).wrapping_add(7) as u8)
        .collect();
    let receiver = tokio::spawn({
        let to = Arc::clone(to);
        async move {
            let mut accepted = to.accept_resource().await.unwrap();
            accepted.session.receive().await.unwrap()
        }
    });
    let config = ResourceTransferConfig {
        timeout: Duration::from_secs(60),
        retry_interval: Duration::from_secs(2),
        request_window: 1,
    };
    let mode = from
        .send_payload_with_config(dest, identity, &payload, config)
        .await
        .expect("receiver proves the resource");
    assert_eq!(mode, PayloadMode::Resource);
    assert_eq!(receiver.await.unwrap(), ReceivedPayload::Resource(payload));
}

/// The whole exchange between `a` and `b`, within `limit`.
async fn exchange(
    (a, a_names): (&Arc<Endpoint>, &[DestinationName; 2]),
    (b, b_names): (&Arc<Endpoint>, &[DestinationName; 2]),
    limit: Duration,
) -> Exchange {
    let start = Instant::now();
    tokio::time::timeout(limit, async {
        let (b_heard, a_heard) = tokio::join!(hear(a, a_names, b), hear(b, b_names, a));
        let announces = start.elapsed();
        echo(a, a_heard[b"echo".as_slice()], "a to b").await;
        let link_a_to_b = start.elapsed();
        echo(b, b_heard[b"echo".as_slice()], "b to a").await;
        let link_b_to_a = start.elapsed();
        resource(a, b, a_heard[b"resource".as_slice()]).await;
        let resource_a_to_b = start.elapsed();
        resource(b, a, b_heard[b"resource".as_slice()]).await;
        Exchange {
            announces,
            link_a_to_b,
            link_b_to_a,
            resource_a_to_b,
            resource_b_to_a: start.elapsed(),
        }
    })
    .await
    .expect("exchange within its limit")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: reseeds from and builds tunnels on the public I2P network"]
async fn two_endpoints_exchange_over_an_embedded_emissary() {
    let storage = std::env::var_os("MURM_I2P_STORAGE").expect("set MURM_I2P_STORAGE");

    // Positive control: the same exchange over loopback TCP, held at the 500-byte protocol
    // MTU the I2P interface uses, so both runs packetize alike and the payload is a Resource
    // in each.
    let (a, a_names) = node(0x31, "tcp-a");
    let (b, b_names) = node(0x32, "tcp-b");
    let policy = ListenPolicy {
        hw_mtu: Some(HwMtu::Protocol),
        ..ListenPolicy::default()
    };
    let listener = a
        .listen_tcp_with("127.0.0.1:0".parse().unwrap(), policy)
        .await
        .unwrap();
    let client = TcpClient {
        hw_mtu: HwMtu::Protocol,
        ..TcpClient::new("127.0.0.1", listener.local_addr().port())
    };
    b.attach_tcp(client).await.unwrap();
    let control = exchange((&a, &a_names), (&b, &b_names), Duration::from_secs(60)).await;
    println!("control (tcp): {control:?}");

    let started = Instant::now();
    let router = EmbeddedRouter::start(RouterConfig::new(storage, Transit::Off, false))
        .await
        .expect("router starts");
    let router_up = started.elapsed();
    let report = router.report().clone();
    println!("router: {report:?} in {router_up:?}");
    let bridge = router.bridge();
    let key = bridge
        .generate(truchman::SIGNATURE_ED25519)
        .await
        .expect("bridge generates a destination");

    let (c, c_names) = node(0x41, "i2p-c");
    let (d, d_names) = node(0x42, "i2p-d");
    c.listen_i2p(I2pListen::new(bridge, key.clone())).unwrap();
    let peer = d
        .attach_i2p_peer(I2pPeer::new(bridge, "I2P", key.destination().to_base64()))
        .unwrap();
    let i2p = exchange((&c, &c_names), (&d, &d_names), Duration::from_secs(600)).await;
    assert!(d.interface_online(peer));
    println!("i2p: {i2p:?}");

    if let Some(path) = std::env::var_os("MURM_I2P_RECEIPT") {
        let ms = |d: Duration| d.as_millis();
        let row = |e: &Exchange| {
            format!(
                "{{\"announces_ms\":{},\"link_a_to_b_ms\":{},\"link_b_to_a_ms\":{},\"resource_a_to_b_ms\":{},\"resource_b_to_a_ms\":{}}}",
                ms(e.announces),
                ms(e.link_a_to_b),
                ms(e.link_b_to_a),
                ms(e.resource_a_to_b),
                ms(e.resource_b_to_a)
            )
        };
        let json = format!(
            "{{\"router\":{{\"stored_routers\":{},\"reseeded\":{},\"start_ms\":{},\"transit\":\"off\",\"publish\":false}},\"destination_b32\":\"{}\",\"control_tcp\":{},\"i2p\":{}}}\n",
            report.stored_routers,
            report
                .reseeded
                .map_or("null".to_string(), |n| n.to_string()),
            ms(router_up),
            key.destination().b32(),
            row(&control),
            row(&i2p)
        );
        std::fs::write(path, json).unwrap();
    }
    router.shutdown().await;
}
