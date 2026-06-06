/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The misfin adapter: a mailbox as conversations, gemmail as messages.
//!
//! A [`MisfinAdapter`] reads the receive server's [`MailboxStore`](misfin::MailboxStore)
//! for one mailbox and groups its mail by correspondent: each sender is a
//! conversation, each received gemmail a message (its gemtext body rides nematic,
//! its first heading is the subject).
//!
//! ## Read-only for now
//!
//! This adapter surfaces received mail. Sending a misfin message rides errand's
//! transport plus a vault-derived client certificate (see the comms-shell plan's
//! P1/P2), which the pane wires; here [`send`](ProtocolAdapter::send) reports
//! `Unsupported`.
//!
//! ## Correspondents are keyed by what the server knows
//!
//! The v1 server stores a sender's certificate fingerprint, not yet its claimed
//! `mailbox@host` (that needs a certificate-DN parse, deferred in the server's
//! P3b′). So a correspondent is grouped by `sender_address` when present and
//! falls back to the fingerprint otherwise.

use async_trait::async_trait;

use misfin::{parse_gemmail, MailboxStore, ReceivedMessage};

use crate::adapter::{AdapterError, ProtocolAdapter};
use crate::model::{
    Conversation, ConversationId, Direction, Draft, Identity, Message, MessageBody, MessageId,
    ProtocolKind,
};

/// Presents one misfin mailbox as comms conversations.
pub struct MisfinAdapter {
    identity: Identity,
    mailbox: String,
    store: MailboxStore,
}

impl MisfinAdapter {
    /// Read `mailbox` (an addr-spec, `mailbox@host`) from `store`, presenting it
    /// under the local `identity`.
    pub fn new(identity: Identity, mailbox: impl Into<String>, store: MailboxStore) -> Self {
        Self {
            identity,
            mailbox: mailbox.into(),
            store,
        }
    }

    /// Every message in this mailbox, newest store order, or a backend error.
    fn all_mail(&self) -> Result<Vec<ReceivedMessage>, AdapterError> {
        self.store
            .list(&self.mailbox)
            .map_err(|error| AdapterError::Backend(error.to_string()))
    }
}

/// The correspondent key for a received message: its sender address when the
/// server resolved one, else the certificate fingerprint.
fn correspondent_key(message: &ReceivedMessage) -> String {
    message
        .sender_address
        .clone()
        .unwrap_or_else(|| message.sender_fingerprint.clone())
}

/// Map a received gemmail into a comms message. The gemtext body is parsed to
/// lift its subject; the body is kept as gemtext for nematic to render.
fn received_to_message(message: &ReceivedMessage) -> Message {
    let gemmail = parse_gemmail(&message.body);
    let display_name = gemmail.sender.and_then(|sender| sender.blurb);
    let mut author = Identity::new(ProtocolKind::Misfin, correspondent_key(message));
    author.display_name = display_name;
    Message {
        id: MessageId(message.seq.to_string()),
        author,
        body: MessageBody::Gemtext(gemmail.body),
        subject: gemmail.subject,
        // The store records seconds; the model is milliseconds.
        timestamp_ms: Some(message.received_at.saturating_mul(1_000)),
        direction: Direction::Incoming,
    }
}

#[async_trait]
impl ProtocolAdapter for MisfinAdapter {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Misfin
    }

    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    async fn conversations(&self) -> Result<Vec<Conversation>, AdapterError> {
        // Group mail by correspondent, preserving first-seen order.
        let mut conversations: Vec<Conversation> = Vec::new();
        for message in self.all_mail()? {
            let key = correspondent_key(&message);
            let activity = message.received_at.saturating_mul(1_000);
            if let Some(existing) = conversations.iter_mut().find(|c| c.id.key == key) {
                existing.unread += 1;
                existing.last_activity_ms =
                    existing.last_activity_ms.max(Some(activity));
            } else {
                let author = received_to_message(&message).author;
                conversations.push(Conversation {
                    id: ConversationId::new(ProtocolKind::Misfin, key),
                    title: author.label().to_string(),
                    participants: vec![author],
                    last_activity_ms: Some(activity),
                    unread: 1,
                });
            }
        }
        // Most recent first; Comms re-sorts the merged set, but be tidy here too.
        conversations.sort_by(|a, b| b.last_activity_ms.cmp(&a.last_activity_ms));
        Ok(conversations)
    }

    async fn messages(&self, conversation: &ConversationId) -> Result<Vec<Message>, AdapterError> {
        let mut messages: Vec<Message> = self
            .all_mail()?
            .iter()
            .filter(|message| correspondent_key(message) == conversation.key)
            .map(received_to_message)
            .collect();
        if messages.is_empty() {
            return Err(AdapterError::NotFound);
        }
        messages.sort_by_key(|message| message.timestamp_ms);
        Ok(messages)
    }

    async fn send(&self, _draft: &Draft) -> Result<MessageId, AdapterError> {
        Err(AdapterError::Unsupported(
            "misfin sending rides errand + a vault identity; wired in the pane".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misfin::{MisfinAddress, MisfinSender};

    fn me() -> Identity {
        Identity::new(ProtocolKind::Misfin, "mark@example.test")
    }

    fn addr(spec: &str) -> MisfinAddress {
        MisfinAddress::parse(spec).unwrap()
    }

    /// A mailbox with mail from two correspondents: Ana (with a resolved address,
    /// and a gemmail carrying her sender line + a subject) and an anonymous sender
    /// known only by fingerprint.
    fn populated_store() -> MailboxStore {
        let store = MailboxStore::in_memory().unwrap();
        let mark = addr("mark@example.test");
        let ana = MisfinSender {
            address: addr("ana@example.test"),
            blurb: Some("Ana".to_string()),
        };
        // Realistic gemmail: a `< sender blurb` line, then a heading + body.
        store
            .store(
                &mark,
                "fp-ana",
                Some(&ana),
                "< ana@example.test Ana\n# Hi\n\nFirst note.",
                1_000,
            )
            .unwrap();
        store
            .store(
                &mark,
                "fp-ana",
                Some(&ana),
                "< ana@example.test Ana\nSecond note.",
                3_000,
            )
            .unwrap();
        // A sender the server couldn't resolve to an address (fingerprint only),
        // whose gemmail carries no sender line.
        store
            .store(&mark, "fp-anon", None, "anon hello", 2_000)
            .unwrap();
        store
    }

    fn adapter() -> MisfinAdapter {
        MisfinAdapter::new(me(), "mark@example.test", populated_store())
    }

    #[tokio::test]
    async fn conversations_group_by_correspondent() {
        let conversations = adapter().conversations().await.unwrap();
        assert_eq!(conversations.len(), 2);
        // Ana's last activity (3000s) is more recent than the anon's (2000s).
        // Keyed by her resolved address, titled by her gemmail blurb.
        assert_eq!(conversations[0].id.key, "ana@example.test");
        assert_eq!(conversations[0].title, "Ana");
        assert_eq!(conversations[0].unread, 2);
        // The anonymous sender is keyed (and titled) by fingerprint.
        assert_eq!(conversations[1].id.key, "fp-anon");
        assert_eq!(conversations[1].title, "fp-anon");
        assert_eq!(conversations[1].unread, 1);
    }

    #[tokio::test]
    async fn messages_for_a_correspondent_parse_subject_and_body() {
        let messages = adapter()
            .messages(&ConversationId::new(ProtocolKind::Misfin, "ana@example.test"))
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        // Time-sorted; the first carries the gemtext heading as its subject.
        assert_eq!(messages[0].subject.as_deref(), Some("Hi"));
        assert!(matches!(messages[0].body, MessageBody::Gemtext(_)));
        assert_eq!(messages[0].direction, Direction::Incoming);
        assert_eq!(messages[0].timestamp_ms, Some(1_000_000));
        assert_eq!(messages[0].author.display_name.as_deref(), Some("Ana"));
    }

    #[tokio::test]
    async fn unknown_correspondent_is_not_found() {
        let result = adapter()
            .messages(&ConversationId::new(ProtocolKind::Misfin, "nobody@nowhere"))
            .await;
        assert_eq!(result, Err(AdapterError::NotFound));
    }

    #[tokio::test]
    async fn sending_is_unsupported_for_now() {
        let result = adapter().send(&Draft::default()).await;
        assert!(matches!(result, Err(AdapterError::Unsupported(_))));
    }
}
