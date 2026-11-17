use super::{AdapterError, ConversationId, ProtocolKind};
use serde::{Deserialize, Serialize};

/// Public identity returned by one stock companion. No synthetic identity is
/// created for the other protocol. Realm names remain host-owned configuration.
/// The 8-bit Meshtastic channel hash alone is not a realm identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "companion", deny_unknown_fields)]
pub enum CompanionScope {
    Meshtastic {
        realm: String,
        node: u32,
        channel: u8,
    },
    MeshCore {
        realm: String,
        key: [u8; 32],
    },
}
impl CompanionScope {
    pub(super) fn protocols(&self) -> Vec<ProtocolKind> {
        match self {
            Self::Meshtastic { .. } => vec![ProtocolKind::Meshtastic],
            Self::MeshCore { .. } => vec![ProtocolKind::MeshCore],
        }
    }
    pub(super) fn realm(&self) -> &str {
        match self {
            Self::Meshtastic { realm, .. } | Self::MeshCore { realm, .. } => realm,
        }
    }
    pub(super) fn valid(&self) -> bool {
        let realm = self.realm();
        !realm.is_empty()
            && realm.len() <= 128
            && realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
    pub(super) fn local(&self, _protocol: ProtocolKind) -> String {
        match self {
            Self::Meshtastic { node, .. } => format!("{node:08x}"),
            Self::MeshCore { key, .. } => hex::encode(key),
        }
    }
    fn channel(&self) -> u8 {
        match self {
            Self::Meshtastic { channel, .. } => *channel,
            Self::MeshCore { .. } => 0,
        }
    }
    pub(super) fn conversation(
        &self,
        protocol: ProtocolKind,
        peer: &str,
    ) -> Result<ConversationId, AdapterError> {
        if !self.protocols().contains(&protocol) {
            return Err(AdapterError::NotFound);
        }
        let local = self.local(protocol);
        let key = if protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{:02x}/{peer}", self.realm(), self.channel())
        } else {
            format!("{}/{local}/{peer}", self.realm())
        };
        let id = ConversationId::new(protocol, key);
        self.peer(&id)?;
        Ok(id)
    }
    pub(super) fn peer(&self, id: &ConversationId) -> Result<String, AdapterError> {
        if !self.protocols().contains(&id.protocol) {
            return Err(AdapterError::NotFound);
        }
        let local = self.local(id.protocol);
        let prefix = if id.protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{:02x}/", self.realm(), self.channel())
        } else {
            format!("{}/{local}/", self.realm())
        };
        let peer = id.key.strip_prefix(&prefix).ok_or(AdapterError::NotFound)?;
        let length = if id.protocol == ProtocolKind::Meshtastic {
            8
        } else {
            64
        };
        if peer.len() != length
            || !peer
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AdapterError::NotFound);
        }
        Ok(peer.to_owned())
    }
}
