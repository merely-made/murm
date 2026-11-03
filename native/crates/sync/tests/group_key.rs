// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::time::Duration;

use identity::{IdentityProvider, InMemoryProvider};
use muniment::{Backend, MemoryBackend};
use murm_sync::GroupKeySync;
use p2panda_core::{Body, Header, Operation, SigningKey};
use stickleback::{GroupKeyExt, GroupKeyLane, GroupSession, GroupSessionId, group_key_sync_topic};
use transport::{P2pandaTransport, PeerID, sync_overlay_topic};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_group_keys_reconcile_rotate_and_release_their_sessions() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let group = GroupSessionId([0x47; 32]);
        let alice = InMemoryProvider::from_seed([0xa1; 32]);
        let bob = InMemoryProvider::from_seed([0xb2; 32]);
        let a_lane = GroupKeyLane::for_identity(MemoryBackend::new(), group, &alice).unwrap();
        let b_lane = GroupKeyLane::for_identity(MemoryBackend::new(), group, &bob).unwrap();
        let (mut a_session, a_prekey) = GroupSession::new(group, &alice).unwrap();
        let (mut b_session, b_prekey) = GroupSession::new(group, &bob).unwrap();
        a_session.register_prekey(&b_prekey).unwrap();
        b_session.register_prekey(&a_prekey).unwrap();
        a_session.create(&[]).unwrap();
        let welcome = a_session.add(b_session.member()).unwrap();
        b_session
            .process(
                a_lane.stable_author(),
                &welcome.control,
                welcome.direct_for(b_session.member()),
            )
            .unwrap();
        a_lane.author_dispatch(&welcome).await.unwrap();

        // A carrier can offer a correctly signed operation for another group.
        // Eidetic's receiver refuses it before any durable key is written.
        let body = b"wrong group";
        let header = Header::builder().body(body).build(
            &SigningKey::from_bytes(&[0xcc; 32]),
            GroupKeyExt {
                group: [0xff; 32],
                parents: vec![],
            },
        );
        let foreign = Operation {
            hash: header.hash(),
            header,
            body: Some(Body::from_bytes(body)),
        };
        let before = b_lane.sync_store().backend().list("").await.unwrap();
        assert!(b_lane.receiver().accept(&foreign).await.is_err());
        assert_eq!(
            before,
            b_lane.sync_store().backend().list("").await.unwrap()
        );

        let a_transport = P2pandaTransport::builder(alice.master_keypair())
            .gossip()
            .bind()
            .await
            .unwrap();
        let b_transport = P2pandaTransport::builder(bob.master_keypair())
            .gossip()
            .bind()
            .await
            .unwrap();
        let overlay = sync_overlay_topic(group_key_sync_topic(group));
        a_transport
            .add_peer(b_transport.endpoint_addr().await.unwrap())
            .await
            .unwrap();
        b_transport
            .add_peer(a_transport.endpoint_addr().await.unwrap())
            .await
            .unwrap();
        a_transport
            .set_topics(PeerID::from_public_key(bob.master_public_key()), &[overlay])
            .await
            .unwrap();
        b_transport
            .set_topics(
                PeerID::from_public_key(alice.master_public_key()),
                &[overlay],
            )
            .await
            .unwrap();
        let (endpoint, gossip) = a_transport.sync_parts().unwrap();
        let a_joined = a_lane.join(endpoint, gossip).await.unwrap();
        let (endpoint, gossip) = b_transport.sync_parts().unwrap();
        let b_joined = b_lane.join(endpoint, gossip).await.unwrap();

        while b_joined.ops_received() < 1 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        b_lane
            .drain_ready(&mut b_session, |_| Ok::<_, String>(()))
            .await
            .unwrap();
        let update = a_session.update().unwrap();
        a_lane.publish(&a_joined, &update).await.unwrap();
        while b_joined.ops_received() < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let drained = b_lane
            .drain_ready(&mut b_session, |_| Ok::<_, String>(()))
            .await
            .unwrap();
        assert_eq!(drained.applied.len(), 1);
        let ciphertext = a_session.seal_random(b"after rotation").unwrap();
        assert_eq!(b_session.open(&ciphertext).unwrap(), b"after rotation");

        tokio::time::timeout(Duration::from_secs(10), async {
            a_joined.leave_and_wait().await.unwrap();
            b_joined.leave_and_wait().await.unwrap();
            a_transport.close().await.unwrap();
            b_transport.close().await.unwrap();
        })
        .await
        .expect("sessions or endpoints did not release");
    })
    .await
    .expect("group-key peers did not converge within the test budget");
}
