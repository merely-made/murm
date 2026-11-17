//! LXMF delivery destination and announce conventions.

use std::io::Cursor;

use crate::stamp::StampCost;
use retinue::destination::DestinationName;
use retinue::endpoint::Endpoint;
use retinue::hash::AddressHash;
use retinue::identity::Identity;
use rmpv::Value;

pub const DEFAULT_MAX_ANNOUNCE_BYTES: usize = 1024;

/// The application data carried by an `lxmf.delivery` announce.
///
/// The wire value is `[display_name, stamp_cost, supported_features]`, each of the first two
/// nil when absent, with the name as MessagePack binary (`LXMRouter.py` 1042-1058). Stock
/// emits it whenever it announces through its router. Older senders emitted only the first
/// two, and before that a bare UTF-8 name, which still decodes, with no cost.
///
/// We always declare an empty feature list: stock reads an absent or nil list as compression
/// (`SF_COMPRESSION`) supported, and outrider implements none. With neither a name nor a cost
/// there is nothing to say, so the app data is empty.
///
/// Costs use stock's 1/256 grid and global cap of 100. A zero cost means none.
/// LXMF 1.3 appends the exact cost after the rounded-up legacy integer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeliveryAnnounce {
    pub display_name: Option<Vec<u8>>,
    pub stamp_cost: Option<StampCost>,
}

/// The largest finite cost stock can search for.
pub const MAX_STAMP_COST: StampCost = StampCost::MAX;

fn normalize_cost(cost: &Value) -> Option<StampCost> {
    let value = match cost {
        Value::Integer(n) => n.as_f64()?,
        Value::F32(n) => f64::from(*n),
        Value::F64(n) => *n,
        _ => return None,
    };
    StampCost::from_f64(value).filter(|cost| *cost > StampCost::ZERO)
}

impl DeliveryAnnounce {
    pub fn named(display_name: impl Into<Vec<u8>>) -> Self {
        Self {
            display_name: Some(display_name.into()),
            stamp_cost: None,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, AnnounceError> {
        let cost = self.stamp_cost.filter(|cost| *cost > StampCost::ZERO);
        if self.display_name.is_none() && cost.is_none() {
            return Ok(Vec::new());
        }
        let name = self
            .display_name
            .as_ref()
            .map_or(Value::Nil, |name| Value::Binary(name.clone()));
        let legacy_cost = cost.map_or(Value::Nil, |cost| Value::from(cost.ceil()));
        let fractional_cost = cost.map_or(Value::Nil, |cost| Value::F64(cost.to_f64()));
        let mut encoded = Vec::new();
        rmpv::encode::write_value(
            &mut encoded,
            &Value::Array(vec![
                name,
                legacy_cost,
                Value::Array(Vec::new()),
                fractional_cost,
            ]),
        )
        .map_err(|_| AnnounceError::Encode)?;
        if encoded.len() > DEFAULT_MAX_ANNOUNCE_BYTES {
            return Err(AnnounceError::TooLarge);
        }
        Ok(encoded)
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, AnnounceError> {
        Self::decode_bounded(encoded, DEFAULT_MAX_ANNOUNCE_BYTES)
    }

    /// Decode as stock's `display_name_from_app_data` and `stamp_cost_from_app_data` do
    /// (`LXMF.py` 152-186): an array marker selects the structured form, anything else is
    /// a legacy UTF-8 name with no cost. A name that is not UTF-8 binary reads as absent;
    /// NULs and surrounding whitespace are stripped from one that is. A cost that is not an
    /// finite integer or float reads as absent. The fourth element, when numeric,
    /// supersedes the second element as in LXMF 1.3.
    ///
    /// Any app data reads as some delivery announce, so decode only what an `lxmf.delivery`
    /// destination announced, as stock's handler filters by aspect (`Handlers.py` 8-12).
    pub fn decode_bounded(encoded: &[u8], max_bytes: usize) -> Result<Self, AnnounceError> {
        if encoded.len() > max_bytes {
            return Err(AnnounceError::TooLarge);
        }
        match encoded.first() {
            None => Ok(Self::default()),
            Some(0x90..=0x9f | 0xdc) => Self::decode_array(encoded),
            Some(_) => Ok(Self {
                display_name: core::str::from_utf8(encoded).ok().map(|name| name.into()),
                stamp_cost: None,
            }),
        }
    }

    fn decode_array(encoded: &[u8]) -> Result<Self, AnnounceError> {
        let mut cursor = Cursor::new(encoded);
        let value = rmpv::decode::read_value(&mut cursor)
            .map_err(|_| AnnounceError::MalformedMessagePack)?;
        if cursor.position() as usize != encoded.len() {
            return Err(AnnounceError::MalformedMessagePack);
        }
        let Value::Array(parts) = value else {
            return Err(AnnounceError::MalformedMessagePack);
        };
        let display_name = match parts.first() {
            Some(Value::Binary(name)) => core::str::from_utf8(name)
                .ok()
                .map(|name| name.replace('\0', "").trim().as_bytes().to_vec()),
            _ => None,
        };
        // LXMF 1.3's transition announces append the exact fractional cost; the
        // second element is its rounded-up legacy fallback.
        let cost = match parts.get(3) {
            Some(value @ (Value::Integer(_) | Value::F32(_) | Value::F64(_))) => Some(value),
            _ => parts.get(1),
        };
        let stamp_cost = cost.and_then(normalize_cost);
        Ok(Self {
            display_name,
            stamp_cost,
        })
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnnounceError {
    #[error("LXMF delivery announce exceeds the configured byte limit")]
    TooLarge,
    #[error("LXMF delivery announce is not one complete MessagePack value")]
    MalformedMessagePack,
    #[error("LXMF delivery announce could not be encoded")]
    Encode,
}

pub fn delivery_name() -> DestinationName {
    DestinationName::new("lxmf", ["delivery"])
}

pub fn delivery_destination(identity: &Identity) -> AddressHash {
    delivery_name().destination_hash(identity)
}

/// Resolve a message source's identity, asking the network for it when we do not have it.
///
/// A message is only verifiable if we hold the sender's keys, and we hold them only from an
/// announce. Nothing obliges a sender to announce before it sends, so a first message from a
/// stranger is unverifiable through no fault of theirs.
///
/// Refusing it is still right: an unverified message must not be delivered. Refusing
/// *silently* is what was wrong, because it made the refusal permanent — every retry hit the
/// same wall, and from the sender's side the recipient simply never answered. Found against
/// MeshChatX 2.0.1 driving a board on the RNode channel: it sent three times, and all three
/// arrived intact and were dropped.
///
/// So a refusal now asks. A path request for the source's delivery destination is answered
/// with an announce, an announce carries the identity, and the sender's next retry verifies.
/// The request is rate-limited per destination inside `retinue`, so a peer sending traffic we
/// cannot verify cannot make us broadcast once per packet.
pub fn resolve_source(endpoint: &Endpoint, source: AddressHash) -> Option<Identity> {
    resolve_source_with_link(endpoint, source, None)
}

/// Resolve a message source, accepting an identity the sender proved on the link it opened.
///
/// `identified` is what the peer signed as part of link setup, so it is *stronger* evidence
/// than an announce: an announce says some destination exists somewhere, while this is the
/// peer on the other end of this link saying who it is. It is still only accepted when it
/// derives to the exact delivery destination the message names as its source, because
/// IDENTIFY proves who the peer is and says nothing about who the payload claims to be from.
///
/// This is what closes the case the path request could not. A path request only helps if the
/// sender answers it, and a client with transport disabled may simply not.
///
/// The accepted identity is deliberately NOT written into the address book: an IDENTIFY is
/// not an announce — it carries no app_data, no stamp cost, and no claim of reachability —
/// so it authenticates this session and nothing beyond it. The same sender on a new link
/// without an IDENTIFY starts over.
pub fn resolve_source_with_link(
    endpoint: &Endpoint,
    source: AddressHash,
    identified: Option<Identity>,
) -> Option<Identity> {
    if let Some(identity) = endpoint.resolve(source) {
        return Some(identity);
    }
    if let Some(identity) = identified
        && delivery_destination(&identity) == source
    {
        return Some(identity);
    }
    endpoint.request_path(source);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_0_9_6_two_element_announce_still_decodes() {
        let captured = hex::decode("92c40c53746f636b204f7261636c6508").unwrap();
        let announce = DeliveryAnnounce::decode(&captured).unwrap();
        assert_eq!(
            announce.display_name.as_deref(),
            Some(b"Stock Oracle".as_slice())
        );
        assert_eq!(announce.stamp_cost, Some(StampCost::from_integer(8)));
    }

    #[test]
    fn stock_1_1_1_three_element_announce_decodes() {
        // Captured from stock LXMF 1.1.1: [b"Stock Receiver", 8, [0]]. The trailing [0]
        // declares compression support; we read the name and cost and ignore it.
        let captured = hex::decode("93c40e53746f636b205265636569766572089100").unwrap();
        let announce = DeliveryAnnounce::decode(&captured).unwrap();
        assert_eq!(
            announce.display_name.as_deref(),
            Some(b"Stock Receiver".as_slice())
        );
        assert_eq!(announce.stamp_cost, Some(StampCost::from_integer(8)));
    }

    #[test]
    fn stock_1_1_1_nil_stamp_cost_announce_decodes() {
        // [b"Stock Opportunistic Receiver", nil, [0]], captured from stock LXMF 1.1.1.
        let captured =
            hex::decode("93c41c53746f636b204f70706f7274756e6973746963205265636569766572c09100")
                .unwrap();
        let announce = DeliveryAnnounce::decode(&captured).unwrap();
        assert_eq!(announce.stamp_cost, None);
    }

    /// The bug this guards against is the one that broke us: LXMF 1.1.1 appended a third
    /// element and a `!= 2` length check refused every stock announce, so outrider never
    /// learned any sender's keys. Stock parses a four-element announce happily; so do we.
    #[test]
    fn announces_longer_than_we_understand_are_accepted() {
        let future = hex::decode("94c4014e089100 63".replace(' ', "").as_str()).unwrap();
        let announce = DeliveryAnnounce::decode(&future).unwrap();
        assert_eq!(announce.display_name.as_deref(), Some(b"N".as_slice()));
        assert_eq!(announce.stamp_cost, Some(StampCost::from_integer(99)));
    }

    /// Outrider implements no compression. An empty feature list is the only encoding that
    /// says so: stock reads both an absent list and a nil list as compression *supported*.
    #[test]
    fn we_declare_an_empty_feature_list() {
        let encoded = DeliveryAnnounce {
            display_name: Some(b"Stock Oracle".to_vec()),
            stamp_cost: Some(StampCost::from_integer(8)),
        }
        .encode()
        .unwrap();
        assert_eq!(
            hex::encode(&encoded),
            "94c40c53746f636b204f7261636c650890cb4020000000000000"
        );

        let anonymous_cost = DeliveryAnnounce::named("Stock Receiver").encode().unwrap();
        assert_eq!(
            hex::encode(&anonymous_cost),
            "94c40e53746f636b205265636569766572c090c0"
        );
    }

    /// What we emit must survive our own decoder, feature list and all.
    #[test]
    fn our_own_announce_round_trips() {
        let announce = DeliveryAnnounce {
            display_name: Some(b"outrider".to_vec()),
            stamp_cost: Some(StampCost::from_integer(4)),
        };
        let encoded = announce.encode().unwrap();
        assert_eq!(DeliveryAnnounce::decode(&encoded).unwrap(), announce);
    }

    #[test]
    fn absent_application_data_is_an_anonymous_announce() {
        assert_eq!(
            DeliveryAnnounce::decode(&[]).unwrap(),
            DeliveryAnnounce::default()
        );
        assert_eq!(
            DeliveryAnnounce::default().encode().unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn malformed_or_oversized_announces_are_rejected() {
        assert_eq!(
            DeliveryAnnounce::decode(&[0x92, 0xc0]),
            Err(AnnounceError::MalformedMessagePack)
        );
        assert_eq!(
            DeliveryAnnounce::decode(&[0x91, 0xc0, 0xc0]),
            Err(AnnounceError::MalformedMessagePack)
        );
        assert_eq!(
            DeliveryAnnounce::decode_bounded(&[0x92, 0xc4, 0x00, 0xc0], 3),
            Err(AnnounceError::TooLarge)
        );
    }

    /// Stock reads a name it cannot decode as absent and still takes the cost; a peer whose
    /// announce has an odd name stays reachable, and stamped.
    #[test]
    fn odd_names_and_costs_read_as_absent() {
        let decode = |bytes: &[u8]| DeliveryAnnounce::decode(bytes).unwrap();
        let int_name = decode(&[0x92, 0x07, 0x08]);
        assert_eq!(int_name.display_name, None);
        assert_eq!(int_name.stamp_cost, Some(StampCost::from_integer(8)));
        let str_name = decode(&[0x92, 0xa1, b'N', 0x08]);
        assert_eq!(str_name.display_name, None);
        assert_eq!(str_name.stamp_cost, Some(StampCost::from_integer(8)));
        assert_eq!(decode(&[0x92, 0xc0, 0xa1, b'8']).stamp_cost, None);
        assert_eq!(decode(&[0xff, 0xfe]), DeliveryAnnounce::default());
    }

    /// What stock's router announces for a destination registered without a name:
    /// `[nil, 8, [0]]`. The cost must survive, or a stamped send goes out unstamped.
    #[test]
    fn a_nameless_stock_announce_keeps_its_cost() {
        let captured = hex::decode("93c0089100").unwrap();
        let announce = DeliveryAnnounce::decode(&captured).unwrap();
        assert_eq!(announce.display_name, None);
        assert_eq!(announce.stamp_cost, Some(StampCost::from_integer(8)));

        let ours = DeliveryAnnounce {
            display_name: None,
            stamp_cost: Some(StampCost::from_integer(8)),
        };
        assert_eq!(
            hex::encode(ours.encode().unwrap()),
            "94c00890cb4020000000000000"
        );
        assert_eq!(
            DeliveryAnnounce::decode(&ours.encode().unwrap()).unwrap(),
            ours
        );
    }

    #[test]
    fn costs_hold_to_stock_range() {
        let decode = |cost: &[u8]| {
            let mut bytes = vec![0x92, 0xc0];
            bytes.extend_from_slice(cost);
            DeliveryAnnounce::decode(&bytes).unwrap().stamp_cost
        };
        assert_eq!(decode(&[0x00]), None);
        assert_eq!(decode(&[0xff]), None, "negative");
        assert_eq!(decode(&[0xcc, 0xfe]), Some(StampCost::MAX));
        assert_eq!(decode(&[0xcd, 0x01, 0x00]), Some(StampCost::MAX));

        let encode = |cost: u16| {
            DeliveryAnnounce {
                display_name: None,
                stamp_cost: Some(StampCost::from_integer(cost)),
            }
            .encode()
            .unwrap()
        };
        assert_eq!(encode(0), Vec::<u8>::new());
        assert_eq!(hex::encode(encode(255)), "94c06490cb4059000000000000");
    }

    #[test]
    fn legacy_and_short_forms_decode_as_stock_reads_them() {
        assert_eq!(
            DeliveryAnnounce::decode(b"Old Peer").unwrap(),
            DeliveryAnnounce::named("Old Peer")
        );
        assert_eq!(
            DeliveryAnnounce::decode(&[0x90]).unwrap(),
            DeliveryAnnounce::default()
        );
        assert_eq!(
            DeliveryAnnounce::decode(&[0x91, 0xc0]).unwrap(),
            DeliveryAnnounce::default()
        );
    }

    #[test]
    fn names_are_stripped_and_non_utf8_names_are_absent() {
        let padded = hex::decode("92c4072020410042200ac0").unwrap();
        assert_eq!(
            DeliveryAnnounce::decode(&padded)
                .unwrap()
                .display_name
                .as_deref(),
            Some(b"AB".as_slice())
        );
        let invalid = hex::decode("92c402fffe08").unwrap();
        let announce = DeliveryAnnounce::decode(&invalid).unwrap();
        assert_eq!(announce.display_name, None);
        assert_eq!(announce.stamp_cost, Some(StampCost::from_integer(8)));
    }
    #[test]
    fn fractional_fourth_cost_controls_delivery_and_legacy_cost_is_rounded_up() {
        let encoded = |fourth| {
            let mut bytes = Vec::new();
            rmpv::encode::write_value(
                &mut bytes,
                &Value::Array(vec![
                    Value::Nil,
                    Value::from(9),
                    Value::Array(Vec::new()),
                    fourth,
                ]),
            )
            .unwrap();
            bytes
        };
        let announce = DeliveryAnnounce::decode(&encoded(Value::F64(8.5))).unwrap();
        assert_eq!(announce.stamp_cost, StampCost::from_f64(8.5));
        let decoded =
            rmpv::decode::read_value(&mut Cursor::new(announce.encode().unwrap())).unwrap();
        let Value::Array(parts) = decoded else {
            panic!("announce array")
        };
        assert_eq!(parts[1], Value::from(9));
        assert_eq!(parts[3], Value::F64(8.5));
        assert_eq!(
            DeliveryAnnounce::decode(&encoded(Value::Nil))
                .unwrap()
                .stamp_cost,
            Some(StampCost::from_integer(9))
        );
        assert_eq!(
            DeliveryAnnounce::decode(&encoded(Value::F64(f64::NAN)))
                .unwrap()
                .stamp_cost,
            None
        );
    }
}
