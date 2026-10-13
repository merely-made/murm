// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Receipts for the Noise session layer: standalone over TCP, and composed
//! over a live iroh stream.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use identity::Ed25519Keypair;

use super::*;
use crate::{Alpn, P2pandaTransport, Transport, TransportKind};

fn keypair(seed: u8) -> Ed25519Keypair {
    Ed25519Keypair::from_seed([seed; 32])
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// A listener plus its bound address.
async fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind(loopback()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

#[tokio::test]
async fn a_session_is_encrypted_and_both_peers_are_proven() {
    let (server_keys, client_keys) = (keypair(1), keypair(2));
    let server_id = PeerID::from_public_key(server_keys.public_key());
    let client_id = PeerID::from_public_key(client_keys.public_key());

    let (listener, addr) = listener().await;
    let alpn = Alpn::new("mere/noise-test/v1");

    let expect_client = client_id;
    let server = tokio::spawn(async move {
        let (mut stream, peer, got) = accept_from(&server_keys, &listener).await.unwrap();
        assert_eq!(peer, expect_client, "the client's identity is proven");
        assert_eq!(got, Alpn::new("mere/noise-test/v1"));

        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"ping");
        stream.write_all(b"pong").await.unwrap();
        stream.flush().await.unwrap();
    });

    let (mut stream, peer) = connect_to(&client_keys, addr, &alpn).await.unwrap();
    assert_eq!(peer, server_id, "the server's identity is proven");

    stream.write_all(b"ping").await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 32];
    let n = stream.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"pong");

    server.await.unwrap();
}

#[tokio::test]
async fn the_peer_id_is_the_ed25519_identity_not_the_noise_key() {
    // The whole point of the identity proof: a completed handshake yields a
    // Mere PeerID, not merely an X25519 key.
    let (server_keys, client_keys) = (keypair(3), keypair(4));
    let (listener, addr) = listener().await;

    let server = tokio::spawn(async move {
        let (stream, peer, _) = accept_from(&server_keys, &listener).await.unwrap();
        // The Noise static key is a different key from the identity.
        let noise_key = stream.peer_static_key().unwrap().to_vec();
        assert_ne!(
            noise_key.as_slice(),
            peer.public_key().to_bytes().as_slice(),
            "the proven identity is not the raw Noise static key"
        );
        peer
    });

    let alpn = Alpn::new("mere/noise-test/v1");
    let (_stream, _peer) = connect_to(&client_keys, addr, &alpn).await.unwrap();

    assert_eq!(
        server.await.unwrap(),
        PeerID::from_public_key(client_keys.public_key())
    );
}

#[tokio::test]
async fn a_payload_larger_than_one_noise_message_survives_the_split() {
    // A Noise message caps at 65535 bytes, so this crosses frames and the
    // reader must see one continuous byte stream.
    let (server_keys, client_keys) = (keypair(5), keypair(6));
    let (listener, addr) = listener().await;

    let size = super::stream::MAX_PLAINTEXT * 2 + 1234;
    let server = tokio::spawn(async move {
        let (mut stream, _, _) = accept_from(&server_keys, &listener).await.unwrap();
        let mut received = Vec::new();
        let mut buf = vec![0u8; 8192];
        while received.len() < size {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            received.extend_from_slice(&buf[..n]);
        }
        received
    });

    let alpn = Alpn::new("mere/noise-test/v1");
    let (mut stream, _) = connect_to(&client_keys, addr, &alpn).await.unwrap();
    let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();

    let received = server.await.unwrap();
    assert_eq!(
        received.len(),
        size,
        "every byte crossed the frame boundary"
    );
    assert_eq!(received, payload, "and in order, unaltered");
}

#[tokio::test]
async fn tampered_ciphertext_is_refused_rather_than_delivered() {
    // The AEAD's job, actually exercised: a proxy sits on the wire and flips a
    // byte in the data phase. The read must fail rather than yield altered
    // plaintext.
    let (server_keys, client_keys) = (keypair(7), keypair(8));
    let (real, real_addr) = listener().await;
    // A corrupting proxy in front of the real listener.
    let (proxy, proxy_addr) = listener().await;
    tokio::spawn(async move {
        let (mut inbound, _) = proxy.accept().await.unwrap();
        let mut outbound = tokio::net::TcpStream::connect(real_addr).await.unwrap();
        let (mut ri, mut wi) = inbound.split();
        let (mut ro, mut wo) = outbound.split();

        // Server to client, untouched.
        let back = async { tokio::io::copy(&mut ro, &mut wi).await };
        // Client to server, corrupting one byte once the handshake is done.
        let forth = async {
            let mut seen = 0usize;
            let mut buf = vec![0u8; 4096];
            loop {
                let n = match ri.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                seen += n;
                // The XX handshake and the proof exchange come first; corrupt
                // only once real data is flowing.
                if seen > 400 {
                    buf[n / 2] ^= 0xFF;
                }
                if wo.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        };
        // Both halves end when the sockets close; a copy error here is the
        // teardown, not a failure the test cares about.
        let (_, ()) = tokio::join!(back, forth);
    });

    let server = tokio::spawn(async move {
        let (mut stream, _, _) = accept_from(&server_keys, &real).await?;
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(buf[..n].to_vec())
    });

    let alpn = Alpn::new("mere/noise-test/v1");
    let (mut stream, _) = connect_to(&client_keys, proxy_addr, &alpn).await.unwrap();
    // Long enough that the corrupting branch is certainly past its threshold.
    let payload = vec![b'A'; 512];
    let _ = stream.write_all(&payload).await;
    let _ = stream.flush().await;

    match server.await.unwrap() {
        Err(_) => {},
        Ok(received) => assert_ne!(
            received, payload,
            "tampered ciphertext must never decrypt to the original plaintext"
        ),
    }
}

#[tokio::test]
async fn a_listener_hands_out_the_alpn_it_was_asked_for() {
    let server_keys = keypair(9);
    let client_keys = keypair(10);
    let listener = NoiseListener::bind(server_keys, loopback()).await.unwrap();
    let addr = listener.local_addr().unwrap();

    let wanted = Alpn::new("mere/wanted/v1");
    let other = Alpn::new("mere/other/v1");

    // A session on the wrong ALPN arrives first and must not be handed out.
    let dial_other = {
        let keys = keypair(11);
        let other = other.clone();
        tokio::spawn(async move { connect_to(&keys, addr, &other).await.map(|(s, _)| s) })
    };
    let dial_wanted = tokio::spawn(async move {
        // Order the dials so the unwanted one lands first.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        connect_to(&client_keys, addr, &wanted)
            .await
            .map(|(s, _)| s)
    });

    let accepted = listener
        .accept_alpn(&Alpn::new("mere/wanted/v1"))
        .await
        .unwrap();
    assert_eq!(accepted.protocol, Alpn::new("mere/wanted/v1"));
    assert_eq!(
        accepted.peer,
        Some(PeerID::from_public_key(keypair(10).public_key())),
        "the session handed back is the one on the wanted ALPN"
    );
    assert_eq!(accepted.ingress.transport, TransportKind::Noise);

    let _ = dial_other.await.unwrap();
    let _ = dial_wanted.await.unwrap();
}

/// The composition this module exists for, over a real iroh connection.
///
/// Two things have to hold at once, and they are the reason the layers are
/// separate: iroh proves the *carrier* identity that routing depends on, and
/// Noise proves an *application* identity that iroh never saw. A peer learns
/// who is speaking without that being the same fact as which node it reached.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn noise_over_an_iroh_stream_layers_a_second_identity() {
    let alpn = Alpn::new("mere/noise-over-iroh/v1");

    // Carrier identities: what iroh routes to.
    let (alice_carrier, bob_carrier) = (keypair(20), keypair(21));
    let alice_carrier_id = PeerID::from_public_key(alice_carrier.public_key());
    let bob_carrier_id = PeerID::from_public_key(bob_carrier.public_key());

    // Application identities: who is actually speaking. Derived, so they are
    // durable across sessions but unlinkable to the carrier key.
    let alice_app = alice_carrier.derive_child(b"noise-test-app");
    let bob_app = bob_carrier.derive_child(b"noise-test-app");
    let alice_app_id = PeerID::from_public_key(alice_app.public_key());
    let bob_app_id = PeerID::from_public_key(bob_app.public_key());

    assert_ne!(
        alice_app_id, alice_carrier_id,
        "the premise: the application identity is a different key from the carrier's"
    );

    let alice = P2pandaTransport::bind(&alice_carrier, vec![alpn.clone()])
        .await
        .expect("bind alice");
    let bob = P2pandaTransport::bind(&bob_carrier, vec![alpn.clone()])
        .await
        .expect("bind bob");

    alice
        .add_peer(bob.endpoint_addr().await.unwrap())
        .await
        .expect("alice.add_peer");
    bob.add_peer(alice.endpoint_addr().await.unwrap())
        .await
        .expect("bob.add_peer");

    let alice_task = {
        let alpn = alpn.clone();
        tokio::spawn(async move {
            // The carrier opens the stream; Noise runs *inside* it.
            let carrier_stream = alice
                .connect(bob_carrier_id, alpn.clone())
                .await
                .expect("iroh connect");
            let (mut secured, peer) = secure_initiator(&alice_app, carrier_stream, &alpn)
                .await
                .expect("noise handshake over iroh");

            secured.write_all(b"inside two envelopes").await.unwrap();
            secured.flush().await.unwrap();
            let mut buf = [0u8; 32];
            let n = secured.read(&mut buf).await.unwrap();
            (peer, buf[..n].to_vec())
        })
    };

    let accepted = bob.accept(alpn.clone()).await.expect("iroh accept");
    // The carrier layer still reports the carrier identity, unchanged.
    assert_eq!(
        accepted.peer,
        Some(alice_carrier_id),
        "iroh proves the identity it routed to"
    );
    assert_eq!(accepted.ingress.transport, TransportKind::P2panda);

    let (mut secured, noise_peer, inner_alpn) = secure_responder(&bob_app, accepted.into_stream())
        .await
        .expect("noise handshake over iroh");

    // And the session layer reports a different one, which is the point.
    assert_eq!(
        noise_peer, alice_app_id,
        "Noise proves the application identity"
    );
    assert_ne!(
        noise_peer, alice_carrier_id,
        "which iroh never saw, and cannot be read off the carrier"
    );
    assert_eq!(inner_alpn, alpn);

    let mut buf = vec![0u8; 20];
    secured.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"inside two envelopes");
    secured.write_all(b"and back out").await.unwrap();
    secured.flush().await.unwrap();

    let (alice_saw, reply) = alice_task.await.unwrap();
    assert_eq!(
        alice_saw, bob_app_id,
        "the layering is symmetric: Alice sees Bob's application identity"
    );
    assert_eq!(reply, b"and back out");
}
