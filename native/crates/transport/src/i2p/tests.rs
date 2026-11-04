// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn only_canonical_i2p_aliases_and_nonzero_hops_are_accepted() {
    for address in [
        "127.0.0.1:7656",
        "https://example.com",
        "node-ticket",
        "example.i2p",
        "a.b32.i2p",
    ] {
        assert!(I2pAddress::parse(address).is_err());
    }
    let good = format!("{}.b32.i2p", "a".repeat(52));
    assert!(I2pAddress::parse(&good).is_ok());
    assert!(I2pAddress::parse(&good.to_uppercase()).is_err());
    assert!(I2pAddress::parse(&format!("{}b.b32.i2p", "a".repeat(51))).is_err());
    for (inbound, outbound, quantity) in [(0, 2, 2), (2, 0, 2), (9, 2, 2), (2, 2, 0)] {
        assert!(TunnelProfile::new(inbound, outbound, quantity, true).is_err());
    }
    let options = TunnelProfile::new(2, 3, 2, true).unwrap().options().options;
    assert!(options.contains(&("inbound.length".into(), "2".into())));
    assert!(options.contains(&("outbound.lengthVariance".into(), "0".into())));
    assert!(options.contains(&("i2cp.dontPublishLeaseSet".into(), "false".into())));
}

#[cfg(feature = "test-identity")]
#[tokio::test]
async fn parent_session_termination_poison_streams_and_restart_needs_a_new_alias() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture::SamFixture::start().await;
    let profile = TunnelProfile::new(2, 2, 2, true).unwrap();
    let server = I2pSession::create(
        fixture.bridge(),
        Key::Transient,
        profile,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let client = I2pSession::create(
        fixture.bridge(),
        Key::Transient,
        profile,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let address = server.address().unwrap();
    let accepted = server.clone();
    let task = tokio::spawn(async move { accepted.accept().await.unwrap() });
    tokio::task::yield_now().await;
    let mut stream = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match client.connect(&address).await {
                Ok(stream) => break stream,
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .unwrap();
    let mut inbound = task.await.unwrap();
    stream.write_all(b"fixture").await.unwrap();
    let mut bytes = [0; 7];
    inbound.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"fixture");
    fixture.terminate(&address);
    tokio::time::timeout(Duration::from_secs(1), server.closed())
        .await
        .unwrap();
    assert!(inbound.read_u8().await.is_err());
    // Repeated polls return refusal, never poll a completed close future again.
    assert!(inbound.read_u8().await.is_err());
    assert!(server.address().is_err());
    let replacement = I2pSession::create(
        fixture.bridge(),
        Key::Transient,
        profile,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_ne!(replacement.address().unwrap(), address);
    assert!(client.connect(&address).await.is_err());
    assert_eq!(fixture.profiles().len(), 3);
    drop((stream, inbound, server, client, replacement));
    fixture.shutdown().await;
}
