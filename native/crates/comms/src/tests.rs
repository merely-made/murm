/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Headless tests for the unified model, driven by [`InMemoryAdapter`] backends —
//! no host, no network.

use async_trait::async_trait;

use crate::Comms;
use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::in_memory::{InMemoryAdapter, sample_message};
use crate::model::{
    Conversation, ConversationId, Draft, Identity, Message, MessageId, ProtocolKind,
};

fn identity(protocol: ProtocolKind, address: &str) -> Identity {
    Identity::new(protocol, address)
}

fn conversation(
    protocol: ProtocolKind,
    key: &str,
    title: &str,
    last_activity_ms: Option<u64>,
) -> Conversation {
    Conversation {
        id: ConversationId::new(protocol, key),
        title: title.to_string(),
        participants: Vec::new(),
        last_activity_ms,
        unread: 0,
    }
}

/// A misfin-style receive-only backend with two conversations, and a murm-style
/// sending backend with one. The murm conversation is the most recent.
fn two_backends() -> Comms {
    let peer = identity(ProtocolKind::Misfin, "ana@example.test");
    let misfin = InMemoryAdapter::new(
        ProtocolKind::Misfin,
        identity(ProtocolKind::Misfin, "mark@example.test"),
    )
    .receive_only()
    .with_conversation(
        conversation(ProtocolKind::Misfin, "ana@example.test", "Ana", Some(1_000)),
        vec![sample_message("m1", peer.clone(), "first mail", 1_000)],
    )
    .with_conversation(
        conversation(ProtocolKind::Misfin, "bo@example.test", "Bo", Some(3_000)),
        vec![],
    );

    let murm = InMemoryAdapter::new(
        ProtocolKind::Murm,
        identity(ProtocolKind::Murm, "cabal-self"),
    )
    .with_conversation(
        conversation(
            ProtocolKind::Murm,
            "cabal-abc",
            "Project cabal",
            Some(5_000),
        ),
        vec![sample_message(
            "p1",
            identity(ProtocolKind::Murm, "peer-key"),
            "hi over gossip",
            5_000,
        )],
    );

    Comms::new()
        .with_adapter(Box::new(misfin))
        .with_adapter(Box::new(murm))
}

#[tokio::test]
async fn inbox_merges_and_recency_sorts_across_backends() {
    let inbox = two_backends().inbox().await;
    assert!(inbox.failures.is_empty());
    let titles: Vec<&str> = inbox
        .conversations
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    // Most recent first: murm (5000), misfin Bo (3000), misfin Ana (1000).
    assert_eq!(titles, vec!["Project cabal", "Bo", "Ana"]);
}

#[tokio::test]
async fn messages_route_to_the_owning_backend() {
    let comms = two_backends();

    let murm_msgs = comms
        .messages(&ConversationId::new(ProtocolKind::Murm, "cabal-abc"))
        .await
        .unwrap();
    assert_eq!(murm_msgs.len(), 1);
    assert_eq!(murm_msgs[0].body.text(), "hi over gossip");

    let misfin_msgs = comms
        .messages(&ConversationId::new(
            ProtocolKind::Misfin,
            "ana@example.test",
        ))
        .await
        .unwrap();
    assert_eq!(misfin_msgs[0].body.text(), "first mail");
}

#[tokio::test]
async fn messages_for_unknown_conversation_is_not_found() {
    let comms = two_backends();
    let result = comms
        .messages(&ConversationId::new(ProtocolKind::Murm, "no-such-cabal"))
        .await;
    assert_eq!(result, Err(AdapterError::NotFound));
}

#[tokio::test]
async fn send_routes_by_protocol_and_respects_capability() {
    let comms = two_backends();

    // Murm backend can send.
    let sent = comms
        .send(&Draft::reply_to(ConversationId::new(
            ProtocolKind::Murm,
            "cabal-abc",
        )))
        .await
        .unwrap();
    assert_eq!(sent, MessageId("cabal-abc:cabal-self".to_string()));

    // Misfin backend here is receive-only.
    let receive_only = comms
        .send(&Draft::reply_to(ConversationId::new(
            ProtocolKind::Misfin,
            "ana@example.test",
        )))
        .await;
    assert!(matches!(receive_only, Err(AdapterError::Unsupported(_))));
}

#[tokio::test]
async fn operations_on_an_unregistered_protocol_are_unsupported() {
    // A Comms with only a murm adapter: misfin operations have no backend.
    let comms = Comms::new().with_adapter(Box::new(InMemoryAdapter::new(
        ProtocolKind::Murm,
        identity(ProtocolKind::Murm, "self"),
    )));
    let result = comms
        .messages(&ConversationId::new(ProtocolKind::Misfin, "x@y"))
        .await;
    assert!(matches!(result, Err(AdapterError::Unsupported(_))));
}

#[tokio::test]
async fn identities_lists_one_per_backend() {
    let ids = two_backends().identities();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0].protocol, ProtocolKind::Misfin);
    assert_eq!(ids[1].protocol, ProtocolKind::Murm);
}

/// An adapter that always fails, to exercise the inbox's failure surfacing.
struct FailingAdapter;

#[async_trait]
impl ProtocolAdapter for FailingAdapter {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Misfin
    }
    fn identity(&self) -> Identity {
        identity(ProtocolKind::Misfin, "down@example.test")
    }
    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        Err(AdapterError::Backend("server unreachable".to_string()))
    }
    async fn messages(&self, _: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        Err(AdapterError::Backend("server unreachable".to_string()))
    }
    async fn send(&self, _: &Draft) -> Result<MessageId, AdapterError> {
        Err(AdapterError::Backend("server unreachable".to_string()))
    }
}

#[tokio::test]
async fn inbox_surfaces_a_failing_backend_without_dropping_others() {
    let murm = InMemoryAdapter::new(ProtocolKind::Murm, identity(ProtocolKind::Murm, "self"))
        .with_conversation(
            conversation(
                ProtocolKind::Murm,
                "cabal-abc",
                "Project cabal",
                Some(5_000),
            ),
            vec![],
        );
    let comms = Comms::new()
        .with_adapter(Box::new(FailingAdapter))
        .with_adapter(Box::new(murm));

    let inbox = comms.inbox().await;
    // The healthy backend's conversation still loads.
    assert_eq!(inbox.conversations.len(), 1);
    assert_eq!(inbox.conversations[0].title, "Project cabal");
    // The failure is reported, not swallowed.
    assert_eq!(inbox.failures.len(), 1);
    assert_eq!(inbox.failures[0].protocol, ProtocolKind::Misfin);
    assert!(matches!(inbox.failures[0].error, AdapterError::Backend(_)));
}
