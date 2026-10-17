use super::{AdapterError, ConversationId, ProtocolKind};
use serde::{Deserialize, Serialize};

/// A caller-owned realm binds channel configuration and public device identities.
/// The 8-bit Meshtastic channel hash alone is not a realm identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioScope {
    pub realm: String,
    pub meshtastic_node: u32,
    pub meshtastic_channel: u8,
    pub meshcore_key: [u8; 32],
}
impl RadioScope {
    pub(super) fn local(&self, protocol: ProtocolKind) -> String {
        match protocol {
            ProtocolKind::Meshtastic => format!("{:08x}", self.meshtastic_node),
            ProtocolKind::MeshCore => hex::encode(self.meshcore_key),
            _ => unreachable!("radio adapter protocol"),
        }
    }
    pub fn meshtastic(&self, peer: u32) -> ConversationId {
        ConversationId::new(
            ProtocolKind::Meshtastic,
            format!(
                "{}/{:08x}/{:02x}/{peer:08x}",
                self.realm, self.meshtastic_node, self.meshtastic_channel
            ),
        )
    }
    pub fn meshcore(&self, peer: [u8; 32]) -> ConversationId {
        ConversationId::new(
            ProtocolKind::MeshCore,
            format!(
                "{}/{}/{}",
                self.realm,
                hex::encode(self.meshcore_key),
                hex::encode(peer)
            ),
        )
    }
    pub(super) fn peer(&self, id: &ConversationId) -> Result<String, AdapterError> {
        if !matches!(
            id.protocol,
            ProtocolKind::Meshtastic | ProtocolKind::MeshCore
        ) {
            return Err(AdapterError::NotFound);
        }
        let local = self.local(id.protocol);
        let prefix = match id.protocol {
            ProtocolKind::Meshtastic => {
                format!("{}/{local}/{:02x}/", self.realm, self.meshtastic_channel)
            }
            ProtocolKind::MeshCore => format!("{}/{local}/", self.realm),
            _ => return Err(AdapterError::NotFound),
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

/// Public identity returned by one stock companion. No synthetic identity is
/// created for the other protocol. Realm names remain host-owned configuration.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum Scope {
    Resident(RadioScope),
    Companion(CompanionScope),
}
impl Scope {
    pub(super) fn protocols(&self) -> Vec<ProtocolKind> {
        match self {
            Self::Resident(_) => vec![ProtocolKind::Meshtastic, ProtocolKind::MeshCore],
            Self::Companion(CompanionScope::Meshtastic { .. }) => vec![ProtocolKind::Meshtastic],
            Self::Companion(CompanionScope::MeshCore { .. }) => vec![ProtocolKind::MeshCore],
        }
    }
    pub(super) fn realm(&self) -> &str {
        match self {
            Self::Resident(s) => &s.realm,
            Self::Companion(
                CompanionScope::Meshtastic { realm, .. } | CompanionScope::MeshCore { realm, .. },
            ) => realm,
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
    pub(super) fn local(&self, protocol: ProtocolKind) -> String {
        match self {
            Self::Resident(s) => s.local(protocol),
            Self::Companion(CompanionScope::Meshtastic { node, .. }) => format!("{node:08x}"),
            Self::Companion(CompanionScope::MeshCore { key, .. }) => hex::encode(key),
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
        let channel = match self {
            Self::Resident(s) => s.meshtastic_channel,
            Self::Companion(CompanionScope::Meshtastic { channel, .. }) => *channel,
            _ => 0,
        };
        let local = self.local(protocol);
        let key = if protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{channel:02x}/{peer}", self.realm())
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
        let channel = match self {
            Self::Resident(s) => s.meshtastic_channel,
            Self::Companion(CompanionScope::Meshtastic { channel, .. }) => *channel,
            _ => 0,
        };
        let prefix = if id.protocol == ProtocolKind::Meshtastic {
            format!("{}/{local}/{channel:02x}/", self.realm())
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
