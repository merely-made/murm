// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use identity::delegation::Issue;
use identity::{IdentityProvider, InMemoryProvider};
use insigne::delegation::{
    CapabilityScope, DelegationCertificate, DelegationParent, SignedDelegationCertificate,
};
use murm_ocapn::{
    Bootstrap, BoxFuture, CallContext, CallError, HostObject, PeerLocator, SessionAuthority,
    SessionConfig, Value, attach,
};
use murm_ocapn_transport::{CONNECT, Credentials, GEMOT_DOMAIN, GEMOT_SERVICE, NativeService};
use notochord::{
    AdmittedPrincipal, LocalNetworkPolicy, NetworkId, ProfileRef, RevocationLedger, ServiceAccess,
    ServiceRule, TrustedRoot,
};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use transport::security::SetupLimits;
use transport::{Alpn, P2pandaTransport, Transport};

const NETWORK: NetworkId = NetworkId([54; 32]);
const ROOT: [u8; 32] = [55; 32];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_that_never_discloses_its_first_stream_releases_ingress_capacity() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let server_id = InMemoryProvider::from_seed([81; 32]);
        let client_id = InMemoryProvider::from_seed([82; 32]);
        let server_key = server_id.derived_keypair(b"ingress-test").unwrap();
        let client_key = client_id.derived_keypair(b"ingress-test").unwrap();
        let server = P2pandaTransport::builder_for(&server_key)
            .bind()
            .await
            .unwrap();
        let client = P2pandaTransport::builder_for(&client_key)
            .bind()
            .await
            .unwrap();
        client
            .add_peer(server.endpoint_addr().await.unwrap())
            .await
            .unwrap();
        let service = NativeService::gemot(1, SetupLimits::default());
        let listener = service
            .register(&server, 1, Duration::from_millis(100))
            .await
            .unwrap();
        // QUIC opens this stream lazily. Retaining it without writing forces
        // the route's first-stream wait, before any setup/admission guard exists.
        let _silent = client
            .connect(server.local_peer_id(), service.protocol().clone())
            .await
            .unwrap();
        while listener.ingress_stats().peak_pending == 0 {
            tokio::task::yield_now().await;
        }
        while listener.ingress_stats().pending != 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(listener.ingress_stats().peak_pending, 1);
        assert_eq!(service.stats().established, 0);
        assert_eq!(service.stats().pending, 0);
    })
    .await
    .expect("first-stream ingress wait is bounded");
}

fn profile() -> ProfileRef {
    ProfileRef {
        id: "murm.ocapn-test".into(),
        revision: 1,
    }
}

struct Echo {
    subject: [u8; 32],
}
impl HostObject for Echo {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let authority = context
                .authority::<AdmittedPrincipal>()
                .ok_or_else(|| CallError::message("missing admitted principal"))?;
            if authority.subject != self.subject
                || authority.action.path != GEMOT_SERVICE
                || authority.action.action != CONNECT
            {
                return Err(CallError::message("wrong admitted principal"));
            }
            Ok(Value::List(args))
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_admission_and_captp_reuse_the_endpoint_alongside_another_service() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let server_identity = InMemoryProvider::from_seed([51; 32]);
        let client_identity = InMemoryProvider::from_seed([52; 32]);
        let server_key = server_identity
            .derived_keypair(b"test/scoped-native-endpoint")
            .unwrap();
        let client_key = client_identity
            .derived_keypair(b"test/scoped-native-endpoint")
            .unwrap();
        let service = NativeService::gemot(2, SetupLimits::default());
        let other = Alpn::new("mere/ocapn-endpoint-reuse-test/v1");
        let server = Arc::new(
            P2pandaTransport::builder_for(&server_key)
                .alpns(vec![other.clone()])
                .bind()
                .await
                .unwrap(),
        );
        let client = Arc::new(
            P2pandaTransport::builder_for(&client_key)
                .alpns(vec![other.clone()])
                .bind()
                .await
                .unwrap(),
        );
        client
            .add_peer(server.endpoint_addr().await.unwrap())
            .await
            .unwrap();
        server
            .add_peer(client.endpoint_addr().await.unwrap())
            .await
            .unwrap();
        let listener = service
            .register(&server, 2, Duration::from_secs(1))
            .await
            .unwrap();
        let server_peer = server.local_peer_id();
        let mut policy = LocalNetworkPolicy::closed(NETWORK);
        policy.accepted_profiles.push(profile());
        policy.trusted_roots.push(TrustedRoot {
            authority: ROOT,
            issuer: server_identity.master_public_key().to_bytes(),
        });
        policy.services.insert(
            GEMOT_SERVICE.into(),
            ServiceRule::new(
                ServiceAccess::MemberOnly,
                GEMOT_DOMAIN,
                [CONNECT],
                false,
                Some(2),
            ),
        );
        let grant = SignedDelegationCertificate::issue(
            &server_identity,
            DelegationCertificate::new(
                DelegationParent::Root(ROOT),
                server_identity.master_public_key().to_bytes(),
                client_identity.master_public_key().to_bytes(),
                CapabilityScope {
                    domain: GEMOT_DOMAIN.into(),
                    resource: NETWORK.0.to_vec(),
                    path_prefix: GEMOT_SERVICE.into(),
                    actions: [CONNECT.to_owned()].into_iter().collect(),
                },
                0,
                0,
                Some(100),
                0,
                [56; 32],
            ),
        )
        .unwrap();
        let ledger = RevocationLedger::new();
        let (admitted, connected) = tokio::join!(
            listener.accept(&policy, &ledger, || 1, 0),
            service.connect(
                &client,
                server_peer,
                Credentials {
                    identity: &client_identity,
                    network: NETWORK,
                    profile: profile(),
                    nonce: [57; 32],
                    delegations: vec![grant]
                }
            ),
        );
        let admitted = admitted.unwrap();
        let connected = connected.unwrap();
        assert_eq!(service.stats().pending, 0);
        assert_eq!(listener.ingress_stats().limit, 2);
        assert_eq!(listener.ingress_stats().pending, 0);
        assert_eq!(service.stats().admitted, 2);
        assert_eq!(admitted.secure_channel, connected.secure_channel);
        assert_eq!(
            admitted.secure_channel.initiator,
            client.local_peer_id().to_bytes()
        );
        assert_ne!(
            admitted.secure_channel.initiator,
            client_identity.master_public_key().to_bytes()
        );
        assert_eq!(
            admitted.secure_channel.responder,
            server.local_peer_id().to_bytes()
        );
        assert_ne!(
            admitted.secure_channel.responder,
            server_identity.master_public_key().to_bytes()
        );
        assert_eq!(
            admitted.session.principal.subject,
            client_identity.master_public_key().to_bytes()
        );
        let authority = admitted.session.principal.clone();
        let server_locator = PeerLocator::new("murm-iroh", "test-server");
        let client_locator = PeerLocator::new("murm-iroh", "test-client");
        let mut server_config =
            SessionConfig::new(server_locator.clone(), SessionAuthority::new(authority));
        server_config.expected_remote = Some(client_locator.clone());
        let mut client_config = SessionConfig::new(client_locator, SessionAuthority::new(()));
        client_config.expected_remote = Some(server_locator);
        let mut bootstrap = Bootstrap::new();
        bootstrap
            .register(
                b"test-only-echo".to_vec(),
                Arc::new(Echo {
                    subject: client_identity.master_public_key().to_bytes(),
                }),
            )
            .unwrap();
        let (reader, writer) = tokio::io::split(admitted.into_stream());
        let (server_handle, server_driver) =
            attach(reader, writer, server_config, bootstrap).unwrap();
        let (reader, writer) = tokio::io::split(connected.stream);
        let (client_handle, client_driver) =
            attach(reader, writer, client_config, Bootstrap::new()).unwrap();
        let server_task = tokio::spawn(server_driver.run());
        let client_task = tokio::spawn(client_driver.run());
        client_handle.wait_ready().await.unwrap();
        let target = client_handle.fetch(b"test-only-echo").await.unwrap();
        let answer = client_handle
            .call(&target.target(), vec![Value::String("live".into())])
            .await
            .unwrap();
        match answer.resolve().await.unwrap() {
            Value::List(args) => {
                assert!(matches!(args.as_slice(), [Value::String(s)] if s == "live"))
            },
            value => panic!("unexpected echo: {value:?}"),
        }

        // An existing unrelated service remains usable on exactly these endpoints
        // while the admitted CapTP session and all its state remain live.
        let (accepted, outgoing) = tokio::join!(
            async {
                let mut stream = server.accept(other.clone()).await.unwrap().stream;
                assert_eq!(stream.read_u8().await.unwrap(), 42);
                stream.write_u8(43).await.unwrap();
                stream.flush().await.unwrap();
                stream
            },
            async {
                let mut stream = client.connect(server_peer, other.clone()).await.unwrap();
                stream.write_u8(42).await.unwrap();
                stream.flush().await.unwrap();
                assert_eq!(stream.read_u8().await.unwrap(), 43);
                stream
            }
        );
        drop((accepted, outgoing));
        client_handle.close("test complete").await.unwrap();
        client_handle.closed().await;
        server_handle.closed().await;
        let _ = client_task.await.unwrap();
        let _ = server_task.await.unwrap();
        assert_eq!(service.stats().pending, 0);
    })
    .await
    .expect("native endpoint/CapTP lifetime is bounded");
}
