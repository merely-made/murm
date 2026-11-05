// SPDX-License-Identifier: MPL-2.0
//! Bounded opaque advertisements on a host-selected gossip topic. Applications
//! verify authorship, scope, expiry and permissions. Topics are not subject tags.
use crate::Gossip;
use p2panda_core::Topic;
use p2panda_net::gossip::GossipHandle;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio_stream::StreamExt;

#[derive(Clone, Copy, Debug)]
pub struct AdvertisementLimits {
    pub max_bytes: usize,
    pub pending: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdvertisementStatus {
    pub received: u64,
    pub oversized: u64,
    pub dropped: u64,
}
struct Inbox {
    messages: VecDeque<Vec<u8>>,
    status: AdvertisementStatus,
}
pub struct Advertisements {
    handle: GossipHandle,
    limits: AdvertisementLimits,
    inbox: Arc<Mutex<Inbox>>,
    task: tokio::task::JoinHandle<()>,
}
impl Advertisements {
    /// Reuse an existing transport's gossip. The caller explicitly chooses the
    /// rendezvous and public or protected disclosure policy. Confidential topics
    /// must be unpredictable; knowing a topic supplies no application authority.
    pub async fn join(
        gossip: Gossip,
        topic: [u8; 32],
        limits: AdvertisementLimits,
    ) -> Result<Self, String> {
        if limits.max_bytes == 0 || limits.pending == 0 {
            return Err("advertisement limits must be positive".into());
        }
        let handle = gossip
            .stream(Topic::from(topic))
            .await
            .map_err(|e| e.to_string())?;
        let mut receiver = handle.subscribe();
        let inbox = Arc::new(Mutex::new(Inbox {
            messages: VecDeque::new(),
            status: AdvertisementStatus::default(),
        }));
        let output = inbox.clone();
        let task = tokio::spawn(async move {
            while let Some(message) = receiver.next().await {
                let mut state = output.lock().expect("advertisement inbox");
                match message {
                    Ok(bytes) if bytes.len() <= limits.max_bytes => {
                        state.status.received = state.status.received.saturating_add(1);
                        if state.messages.len() == limits.pending {
                            state.messages.pop_front();
                            state.status.dropped = state.status.dropped.saturating_add(1);
                        }
                        state.messages.push_back(bytes);
                    },
                    Ok(_) => state.status.oversized = state.status.oversized.saturating_add(1),
                    Err(_) => state.status.dropped = state.status.dropped.saturating_add(1),
                }
            }
        });
        Ok(Self {
            handle,
            limits,
            inbox,
            task,
        })
    }
    pub async fn publish(&self, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() > self.limits.max_bytes {
            return Err("advertisement byte ceiling".into());
        }
        self.handle.publish(bytes).await.map_err(|e| e.to_string())
    }
    pub fn drain(&self) -> Vec<Vec<u8>> {
        self.inbox
            .lock()
            .expect("advertisement inbox")
            .messages
            .drain(..)
            .collect()
    }
    pub fn status(&self) -> AdvertisementStatus {
        self.inbox.lock().expect("advertisement inbox").status
    }
}
impl Drop for Advertisements {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use identity::{IdentityProvider, InMemoryProvider};
    use std::time::Duration;
    use transport::{P2pandaTransport, PeerID};
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn opaque_advertisements_cross_real_gossip_and_respect_budgets() {
        let a = InMemoryProvider::from_seed([41; 32]);
        let b = InMemoryProvider::from_seed([42; 32]);
        let alice = P2pandaTransport::builder_for(&a.derived_keypair(b"ads").unwrap())
            .gossip()
            .bind()
            .await
            .unwrap();
        let bob = P2pandaTransport::builder_for(&b.derived_keypair(b"ads").unwrap())
            .gossip()
            .bind()
            .await
            .unwrap();
        let topic = [0x9b; 32];
        let aa = alice.endpoint_addr().await.unwrap();
        let ba = bob.endpoint_addr().await.unwrap();
        alice.add_peer(ba.clone()).await.unwrap();
        bob.add_peer(aa.clone()).await.unwrap();
        alice
            .add_topics(PeerID::from_bytes(ba.id.as_bytes()).unwrap(), &[topic])
            .await
            .unwrap();
        bob.add_topics(PeerID::from_bytes(aa.id.as_bytes()).unwrap(), &[topic])
            .await
            .unwrap();
        let limits = AdvertisementLimits {
            max_bytes: 32,
            pending: 2,
        };
        let sender = Advertisements::join(alice.sync_parts().unwrap().1, topic, limits)
            .await
            .unwrap();
        let receiver = Advertisements::join(bob.sync_parts().unwrap().1, topic, limits)
            .await
            .unwrap();
        assert!(sender.publish(vec![0; 33]).await.is_err());
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                sender
                    .publish(b"signed domain hint".to_vec())
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
                if receiver.drain().iter().any(|b| b == b"signed domain hint") {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(receiver.status().received > 0);
    }
}
