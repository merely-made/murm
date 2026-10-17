use super::*;

impl ResidentSession<'_> {
    pub(super) fn received(
        &self,
        conversation: ConversationId,
        id: String,
        author: String,
        text: String,
        timestamp_ms: Option<u64>,
        fact: RadioFact,
    ) -> Result<(), AdapterError> {
        self.received_facts(conversation, id, author, text, timestamp_ms, vec![fact])
    }
    pub(super) fn received_facts(
        &self,
        conversation: ConversationId,
        id: String,
        author: String,
        text: String,
        timestamp_ms: Option<u64>,
        facts: Vec<RadioFact>,
    ) -> Result<(), AdapterError> {
        self.owner.receive(RadioRecord {
            message: Message {
                id: MessageId(id),
                author: Identity::new(conversation.protocol, author),
                body: MessageBody::PlainText(text),
                subject: None,
                timestamp_ms,
                direction: Direction::Incoming,
            },
            conversation,
            delivery: DeliveryStatus::ReceivedByRadio,
            facts,
        })
    }
    pub(super) fn fault<T>(&self, detail: String) -> Result<T, AdapterError> {
        self.owner.lock()?.fault = Some(detail.clone());
        Err(backend(detail))
    }
}
