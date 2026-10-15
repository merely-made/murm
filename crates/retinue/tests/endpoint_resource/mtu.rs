//! Links over TCP negotiate the autoconfigured 16384-byte MTU, and resources use it.

use super::*;

#[tokio::test]
async fn a_tcp_link_takes_the_autoconfigured_mtu_and_carries_a_resource() {
    let server_id = PrivateIdentity::from_secret_bytes(&[0x71; 64]);
    let server = Arc::new(Endpoint::new(server_id.clone()));
    let client = Endpoint::new(PrivateIdentity::from_secret_bytes(&[0x72; 64]));
    let name = DestinationName::new("retinue", ["tcp-mtu"]);
    let destination = name.destination_hash(server_id.public());
    server.register_resource(name.clone(), b"");
    let addr = server
        .listen_tcp("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    client.attach_tcp_client(addr).await.unwrap();
    // The link request leaves by the learned route, so it signals that interface's MTU.
    let learned = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            server.announce(&name, b"");
            if let Ok(Ok(fact)) =
                tokio::time::timeout(Duration::from_millis(200), client.next_announcement()).await
                && fact.destination == destination
            {
                return;
            }
        }
    });
    learned.await.expect("the client learns the route");

    let payload: Vec<u8> = (0..1_048_576_u32)
        .map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let receiver = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            let mut accepted = server.accept_resource().await.unwrap();
            let mtu = accepted.session.mtu();
            (mtu, accepted.session.receive().await.unwrap())
        }
    });
    let mut session = client
        .open_resource(destination, *server_id.public())
        .await
        .unwrap();
    assert_eq!(session.mtu(), 16_384);
    tokio::time::timeout(Duration::from_secs(30), session.publish(&payload))
        .await
        .expect("publish completes")
        .unwrap();
    let (mtu, received) = tokio::time::timeout(Duration::from_secs(10), receiver)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mtu, 16_384);
    assert_eq!(received, ReceivedPayload::Resource(payload));
}
