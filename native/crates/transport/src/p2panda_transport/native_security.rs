// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::P2pandaStream;
use crate::security::{NATIVE_PROFILE, NativeChannelFacts};
use crate::{PeerID, TransportError};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

impl P2pandaStream {
    /// Derive native authenticated evidence for this exact bidirectional stream.
    ///
    /// The exporter context binds the stream ID as well as both role-ordered
    /// identities, selected ALPN and profile/version. A second stream on the
    /// same TLS connection therefore cannot reuse this admission transcript.
    /// Raw exporter material is cleared and never returned as public evidence.
    pub fn native_channel_facts(&self) -> Result<NativeChannelFacts, TransportError> {
        let remote = PeerID::from_bytes(self._connection.remote_id().as_bytes())
            .map_err(|_| TransportError::Backend("native peer identity invalid".into()))?;
        let local_is_initiator = self.send.id().initiator() == self._connection.side();
        let (initiator, responder) = if local_is_initiator {
            (self.local_peer, remote)
        } else {
            (remote, self.local_peer)
        };
        // p2panda authenticates a network-salted hash of the service ALPN in
        // TLS. The registered/dialled service name is backend metadata; bind
        // both its spelling and the actual negotiated TLS value.
        let protocol = self.service_protocol.clone();
        let mut context = Vec::new();
        context.extend_from_slice(&(NATIVE_PROFILE.len() as u16).to_be_bytes());
        context.extend_from_slice(NATIVE_PROFILE);
        context.extend_from_slice(&1u16.to_be_bytes());
        context.extend_from_slice(&(protocol.len() as u16).to_be_bytes());
        context.extend_from_slice(protocol.as_bytes());
        let tls_alpn = self._connection.alpn();
        context.extend_from_slice(&(tls_alpn.len() as u16).to_be_bytes());
        context.extend_from_slice(tls_alpn);
        context.extend_from_slice(&initiator.to_bytes());
        context.extend_from_slice(&responder.to_bytes());
        context.extend_from_slice(&u64::from(self.send.id()).to_be_bytes());
        let mut exported = Zeroizing::new([0u8; 32]);
        self._connection
            .export_keying_material(
                &mut exported[..],
                b"EXPORTER-murm-secure-bistream-v1",
                &context,
            )
            .map_err(|_| TransportError::Backend("native exporter unavailable".into()))?;
        let binding: [u8; 32] = Sha256::digest(&exported[..]).into();
        Ok(NativeChannelFacts {
            initiator,
            responder,
            protocol,
            binding,
            local_is_initiator,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{ServiceSecurityPolicy, SetupGate, SetupLimits};
    use crate::{Alpn, P2pandaTransport, Transport};
    use identity::Ed25519Keypair;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn from_pair(
        connection: iroh::endpoint::Connection,
        send: iroh::endpoint::SendStream,
        recv: iroh::endpoint::RecvStream,
        local_peer: PeerID,
        service_protocol: Alpn,
    ) -> P2pandaStream {
        P2pandaStream {
            local_peer,
            service_protocol,
            send,
            recv,
            _connection: connection,
            shutdown_ack: None,
            shutdown_finished: false,
            shutdown_complete: false,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_binding_is_shared_but_changes_for_another_stream_on_the_same_connection() {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let client_key = Ed25519Keypair::from_seed([81; 32]);
            let server_key = Ed25519Keypair::from_seed([82; 32]);
            let client_id = PeerID::from_public_key(client_key.public_key());
            let server_id = PeerID::from_public_key(server_key.public_key());
            let alpn = Alpn::new("mere/native-security-test/v1");
            let client = P2pandaTransport::bind(&client_key, vec![alpn.clone()])
                .await
                .unwrap();
            let server = P2pandaTransport::bind(&server_key, vec![alpn.clone()])
                .await
                .unwrap();
            client
                .add_peer(server.endpoint_addr().await.unwrap())
                .await
                .unwrap();
            server
                .add_peer(client.endpoint_addr().await.unwrap())
                .await
                .unwrap();
            let mut first_client = client.connect(server_id, alpn.clone()).await.unwrap();
            first_client.write_all(b"a").await.unwrap();
            first_client.flush().await.unwrap();
            let mut first_server = server.accept(alpn.clone()).await.unwrap().stream;
            assert_eq!(first_server.read_u8().await.unwrap(), b'a');
            let first = first_client.native_channel_facts().unwrap();
            let server_first = first_server.native_channel_facts().unwrap();
            assert_eq!(first.protocol(), &alpn);
            assert_eq!(first.binding(), server_first.binding());
            assert_eq!(first.peer(), server_id);
            assert_eq!(server_first.peer(), client_id);
            assert_eq!(first.initiator, client_id);
            assert_eq!(first.responder, server_id);

            let connection = first_client._connection.clone();
            let (send, recv) = connection.open_bi().await.unwrap();
            let mut second_client = from_pair(connection, send, recv, client_id, alpn.clone());
            second_client.write_all(b"b").await.unwrap();
            second_client.flush().await.unwrap();
            let connection = first_server._connection.clone();
            let (send, recv) = connection.accept_bi().await.unwrap();
            let mut second_server = from_pair(connection, send, recv, server_id, alpn.clone());
            assert_eq!(second_server.read_u8().await.unwrap(), b'b');
            assert_eq!(
                first_client._connection.stable_id(),
                second_client._connection.stable_id()
            );
            let second = second_client.native_channel_facts().unwrap();
            assert_eq!(
                second.binding(),
                second_server.native_channel_facts().unwrap().binding()
            );
            assert_ne!(
                first.binding(),
                second.binding(),
                "one TLS connection still has distinct per-stream admission bindings"
            );

            #[cfg(feature = "notochord")]
            {
                use identity::InMemoryProvider;
                use notochord::{
                    NetworkId, ProfileRef, ProofBinding, RequestedAction, SessionHello,
                    TrafficClass,
                };
                let first_channel = crate::security::SessionSecurityFacts::Native(first)
                    .secure_channel_facts()
                    .unwrap();
                let second_channel = crate::security::SessionSecurityFacts::Native(second)
                    .secure_channel_facts()
                    .unwrap();
                let carrier =
                    ProofBinding::initiator(alpn.as_bytes(), Some(client_id.to_bytes()), None);
                let hello = SessionHello::issue_secure(
                    &InMemoryProvider::from_seed([81; 32]),
                    NetworkId([3; 32]),
                    ProfileRef {
                        id: "mere.base".into(),
                        revision: 1,
                    },
                    RequestedAction {
                        domain: "mere.network".into(),
                        path: "/test".into(),
                        action: "read".into(),
                    },
                    TrafficClass::Interactive,
                    [4; 32],
                    &carrier.clone().with_secure_channel(first_channel),
                    vec![],
                )
                .unwrap();
                assert!(
                    !hello.verify_secure_proof(&carrier.with_secure_channel(second_channel)),
                    "same-connection replay on another stream fails"
                );
            }
            let policy =
                ServiceSecurityPolicy::native(alpn, Some(server_id), SetupLimits::default());
            let gate = SetupGate::new(1);
            let (_, facts, guard) = gate.reserve(&policy).unwrap().native(first_client).unwrap();
            assert_eq!(facts.peer(), Some(server_id));
            assert_eq!(gate.pending(), 1);
            guard.admitted().unwrap();
            assert_eq!(gate.pending(), 0);
        })
        .await
        .expect("native binding fixture deadline");
    }
}
