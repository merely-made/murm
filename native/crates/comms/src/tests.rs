// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Headless tests for the unified model, driven by [`InMemoryAdapter`] backends —
//! no host, no network.

use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::Comms;
use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::in_memory::{InMemoryAdapter, sample_message};
use crate::model::{
    Conversation, ConversationId, DeliveryQueueReason, DeliveryStatus, Draft, Identity, Message,
    MessageId, ProtocolKind,
};

fn identity(protocol: ProtocolKind, address: &str) -> Identity {
    Identity::new(protocol, address)
}

#[tokio::test]
async fn untracked_delivery_is_unknown_only_for_an_existing_message() {
    let model = two_backends();
    let inbox = model.inbox().await;
    let conversation = &inbox.conversations[0].id;
    let messages = model.messages(conversation).await.unwrap();
    assert_eq!(
        model
            .delivery_status(conversation, &messages[0].id)
            .await
            .unwrap(),
        DeliveryStatus::Unknown
    );
    assert_eq!(
        model
            .delivery_status(conversation, &MessageId("absent".into()))
            .await,
        Err(AdapterError::NotFound)
    );
    assert!(matches!(
        model
            .delivery_status(
                &ConversationId::new(ProtocolKind::Lxmf, "unregistered"),
                &MessageId("absent".into())
            )
            .await,
        Err(AdapterError::Unsupported(_))
    ));
}

#[test]
fn delivery_statuses_serialize_and_preserve_failure_detail() {
    let status = DeliveryStatus::Failed {
        detail: "radio window expired".to_string(),
    };

    let encoded = serde_json::to_string(&status).unwrap();
    assert_eq!(
        serde_json::from_str::<DeliveryStatus>(&encoded).unwrap(),
        status
    );
}

#[test]
fn delivery_status_default_is_unknown_not_a_read_receipt() {
    assert_eq!(DeliveryStatus::default(), DeliveryStatus::Unknown);
    assert_eq!(DeliveryStatus::default().label(), "delivery unknown");
}

#[test]
fn delivery_status_labels_cover_each_delivery_fact() {
    let cases = [
        (DeliveryStatus::Unknown, "delivery unknown"),
        (
            DeliveryStatus::Queued(DeliveryQueueReason::Offline),
            "offline, queued",
        ),
        (
            DeliveryStatus::Queued(DeliveryQueueReason::ReadyForCarriage),
            "queued for station",
        ),
        (
            DeliveryStatus::Queued(DeliveryQueueReason::WaitingForPeer),
            "queued, waiting for peer",
        ),
        (DeliveryStatus::HandedToRadio, "handed to radio"),
        (
            DeliveryStatus::AcceptedByPropagationNode,
            "accepted by propagation node",
        ),
        (
            DeliveryStatus::FetchedFromPropagationNode,
            "fetched from propagation node",
        ),
        (DeliveryStatus::ReceivedDirect, "received directly"),
        (DeliveryStatus::ReceivedByRadio, "received by radio"),
        (
            DeliveryStatus::ProtocolAcknowledged,
            "protocol acknowledgment observed",
        ),
        (DeliveryStatus::Cancelled, "cancelled"),
        (
            DeliveryStatus::Failed {
                detail: "radio window expired".to_string(),
            },
            "failed",
        ),
    ];

    for (status, label) in cases {
        assert_eq!(status.label(), label);
    }
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

#[derive(Default)]
struct RouteEffects {
    reads: AtomicUsize,
    sends: AtomicUsize,
    deliveries: AtomicUsize,
}

struct RouteProbe {
    label: &'static str,
    effects: Arc<RouteEffects>,
    listed_protocols: Vec<ProtocolKind>,
}

#[async_trait]
impl ProtocolAdapter for RouteProbe {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Murm
    }

    fn identity(&self) -> Identity {
        identity(ProtocolKind::Murm, "same-local-address")
    }

    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        Ok(self
            .listed_protocols
            .iter()
            .map(|protocol| conversation(*protocol, "same-native-key", self.label, Some(1)))
            .collect())
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        assert_eq!(conversation.account_id, None);
        self.effects.reads.fetch_add(1, Ordering::SeqCst);
        if conversation.key != "same-native-key" {
            return Err(AdapterError::NotFound);
        }
        Ok(vec![sample_message(
            self.label,
            identity(ProtocolKind::Murm, "peer"),
            self.label,
            1,
        )])
    }

    async fn send(&self, draft: &Draft) -> Result<MessageId, AdapterError> {
        assert_eq!(
            draft
                .conversation
                .as_ref()
                .and_then(|id| id.account_id.as_ref()),
            None
        );
        self.effects.sends.fetch_add(1, Ordering::SeqCst);
        if draft.conversation.as_ref().map(|id| id.key.as_str()) != Some("same-native-key") {
            return Err(AdapterError::NotFound);
        }
        Ok(MessageId(self.label.to_owned()))
    }

    async fn delivery_status(
        &self,
        conversation: &ConversationId,
        _message: &MessageId,
    ) -> Result<DeliveryStatus, AdapterError> {
        assert_eq!(conversation.account_id, None);
        self.effects.deliveries.fetch_add(1, Ordering::SeqCst);
        if conversation.key != "same-native-key" {
            return Err(AdapterError::NotFound);
        }
        Ok(DeliveryStatus::Failed {
            detail: self.label.to_owned(),
        })
    }
}

fn route_probe(label: &'static str) -> (RouteProbe, Arc<RouteEffects>) {
    let effects = Arc::new(RouteEffects::default());
    (
        RouteProbe {
            label,
            effects: effects.clone(),
            listed_protocols: vec![ProtocolKind::Murm],
        },
        effects,
    )
}

fn two_route_accounts(reverse: bool) -> (Comms, Arc<RouteEffects>, Arc<RouteEffects>) {
    let (alice, alice_effects) = route_probe("alice");
    let (bob, bob_effects) = route_probe("bob");
    let comms = if reverse {
        Comms::new()
            .with_account("bob-account", Box::new(bob))
            .with_account("alice-account", Box::new(alice))
    } else {
        Comms::new()
            .with_account("alice-account", Box::new(alice))
            .with_account("bob-account", Box::new(bob))
    };
    (comms, alice_effects, bob_effects)
}

#[tokio::test]
async fn same_protocol_accounts_route_by_stable_owner_in_either_registration_order() {
    for reverse in [false, true] {
        let (comms, alice, bob) = two_route_accounts(reverse);
        let inbox = comms.inbox().await;
        assert!(inbox.failures.is_empty());
        assert_eq!(inbox.conversations.len(), 2);
        for row in &inbox.conversations {
            let account = row.id.account_id.as_ref().unwrap();
            let expected = match account.0.as_str() {
                "alice-account" => "alice",
                "bob-account" => "bob",
                other => panic!("unexpected account {other}"),
            };
            let messages = comms.messages(&row.id).await.unwrap();
            assert_eq!(messages[0].body.text(), expected);
            let sent = comms.send(&Draft::reply_to(row.id.clone())).await.unwrap();
            assert_eq!(sent.0, expected);
            assert_eq!(
                comms.delivery_status(&row.id, &sent).await.unwrap(),
                DeliveryStatus::Failed {
                    detail: expected.to_owned()
                }
            );
        }
        for effects in [&alice, &bob] {
            assert_eq!(effects.reads.load(Ordering::SeqCst), 1);
            assert_eq!(effects.sends.load(Ordering::SeqCst), 1);
            assert_eq!(effects.deliveries.load(Ordering::SeqCst), 1);
        }
        let accounts = comms.account_identities();
        assert_eq!(accounts.len(), 2);
        assert!(
            accounts
                .iter()
                .all(|identity| identity.identity.address == "same-local-address")
        );
        assert_eq!(
            accounts
                .iter()
                .map(|identity| identity.account_id.0.as_str())
                .collect::<std::collections::HashSet<_>>(),
            ["alice-account", "bob-account"].into_iter().collect()
        );
    }
}

#[tokio::test]
async fn unresolved_or_ambiguous_accounts_refuse_before_backend_effects() {
    let (comms, alice, bob) = two_route_accounts(false);
    let unknown = ConversationId::for_account(ProtocolKind::Murm, "missing", "same-native-key");
    assert!(matches!(
        comms.messages(&unknown).await,
        Err(AdapterError::UnknownAccount(_))
    ));
    assert!(matches!(
        comms.send(&Draft::reply_to(unknown.clone())).await,
        Err(AdapterError::UnknownAccount(_))
    ));
    assert!(matches!(
        comms
            .delivery_status(&unknown, &MessageId("m".into()))
            .await,
        Err(AdapterError::UnknownAccount(_))
    ));

    let wrong_protocol =
        ConversationId::for_account(ProtocolKind::Misfin, "alice-account", "same-native-key");
    assert!(matches!(
        comms.messages(&wrong_protocol).await,
        Err(AdapterError::UnknownAccount(_))
    ));
    let missing_protocol = ConversationId::new(ProtocolKind::Misfin, "same-native-key");
    assert!(matches!(
        comms.messages(&missing_protocol).await,
        Err(AdapterError::Unsupported(_))
    ));

    let legacy = ConversationId::new(ProtocolKind::Murm, "same-native-key");
    assert_eq!(
        comms.messages(&legacy).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    );
    assert!(matches!(
        comms.send(&Draft::reply_to(legacy.clone())).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    ));
    assert_eq!(
        comms.delivery_status(&legacy, &MessageId("m".into())).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    );

    for effects in [&alice, &bob] {
        assert_eq!(effects.reads.load(Ordering::SeqCst), 0);
        assert_eq!(effects.sends.load(Ordering::SeqCst), 0);
        assert_eq!(effects.deliveries.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn duplicate_explicit_account_registration_is_ambiguous() {
    let (first, first_effects) = route_probe("first");
    let (second, second_effects) = route_probe("second");
    let comms = Comms::new()
        .with_account("duplicate", Box::new(first))
        .with_account("duplicate", Box::new(second));
    let id = ConversationId::for_account(ProtocolKind::Murm, "duplicate", "same-native-key");
    assert_eq!(
        comms.messages(&id).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    );
    assert_eq!(
        comms.send(&Draft::reply_to(id.clone())).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    );
    assert_eq!(
        comms.delivery_status(&id, &MessageId("m".into())).await,
        Err(AdapterError::AmbiguousAccount(ProtocolKind::Murm))
    );
    assert_eq!(first_effects.reads.load(Ordering::SeqCst), 0);
    assert_eq!(second_effects.reads.load(Ordering::SeqCst), 0);
    assert_eq!(first_effects.sends.load(Ordering::SeqCst), 0);
    assert_eq!(second_effects.sends.load(Ordering::SeqCst), 0);
    assert_eq!(first_effects.deliveries.load(Ordering::SeqCst), 0);
    assert_eq!(second_effects.deliveries.load(Ordering::SeqCst), 0);
}

#[test]
fn conversation_ids_read_legacy_serde_and_round_trip_account_scope() {
    let legacy: ConversationId =
        serde_json::from_str(r#"{"protocol":"Murm","key":"same-native-key"}"#).unwrap();
    assert_eq!(legacy.account_id, None);
    let scoped =
        ConversationId::for_account(ProtocolKind::Murm, "alice-account", "same-native-key");
    let encoded = serde_json::to_string(&scoped).unwrap();
    assert_eq!(
        serde_json::from_str::<ConversationId>(&encoded).unwrap(),
        scoped
    );
}

#[tokio::test]
async fn inbox_reports_malformed_rows_and_keeps_valid_rows_from_same_account() {
    let (mut probe, _) = route_probe("mixed rows");
    probe.listed_protocols = vec![ProtocolKind::Misfin, ProtocolKind::Murm];
    let inbox = Comms::new()
        .with_account("mixed-account", Box::new(probe))
        .inbox()
        .await;

    assert_eq!(inbox.conversations.len(), 1);
    assert_eq!(inbox.conversations[0].id.protocol, ProtocolKind::Murm);
    assert_eq!(
        inbox.conversations[0].id.account_id,
        Some(crate::AccountId::new("mixed-account"))
    );
    assert_eq!(inbox.failures.len(), 1);
    assert_eq!(
        inbox.failures[0].account_id,
        crate::AccountId::new("mixed-account")
    );
    assert_eq!(inbox.failures[0].protocol, ProtocolKind::Murm);
    assert!(matches!(inbox.failures[0].error, AdapterError::Backend(_)));
}

/// An adapter that always fails, to exercise the inbox's failure surfacing.
struct FailingAdapter {
    protocol: ProtocolKind,
    address: &'static str,
}

#[async_trait]
impl ProtocolAdapter for FailingAdapter {
    fn protocol(&self) -> ProtocolKind {
        self.protocol
    }
    fn identity(&self) -> Identity {
        identity(self.protocol, self.address)
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
        .with_account(
            "misfin-primary",
            Box::new(FailingAdapter {
                protocol: ProtocolKind::Misfin,
                address: "down-primary@example.test",
            }),
        )
        .with_account(
            "misfin-secondary",
            Box::new(FailingAdapter {
                protocol: ProtocolKind::Misfin,
                address: "down-secondary@example.test",
            }),
        )
        .with_adapter(Box::new(murm));

    let inbox = comms.inbox().await;
    // The healthy backend's conversation still loads.
    assert_eq!(inbox.conversations.len(), 1);
    assert_eq!(inbox.conversations[0].title, "Project cabal");
    // The failure is reported, not swallowed.
    assert_eq!(inbox.failures.len(), 2);
    assert!(
        inbox
            .failures
            .iter()
            .all(|failure| failure.protocol == ProtocolKind::Misfin)
    );
    assert!(
        inbox
            .failures
            .iter()
            .all(|failure| matches!(failure.error, AdapterError::Backend(_)))
    );
    assert_eq!(
        inbox
            .failures
            .iter()
            .map(|failure| failure.account_id.0.as_str())
            .collect::<std::collections::HashSet<_>>(),
        ["misfin-primary", "misfin-secondary"].into_iter().collect()
    );
}

#[test]
fn a_misfin_address_names_its_correspondent_and_nothing_else_does() {
    use crate::misfin_correspondent;

    assert_eq!(
        misfin_correspondent("misfin://alice@example.test"),
        Some("alice@example.test")
    );
    assert_eq!(
        misfin_correspondent("MISFIN://alice@example.test/"),
        Some("alice@example.test")
    );
    for other in [
        "gemini://alice@example.test",
        "misfin://example.test",
        "misfin://@example.test",
        "misfin://alice@",
        "misfin://alice@example.test/inbox",
        "alice@example.test",
    ] {
        assert_eq!(misfin_correspondent(other), None, "{other}");
    }
}
