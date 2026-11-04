// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::*;

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
