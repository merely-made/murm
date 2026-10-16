//! The narrow routing-response envelope observed from stock 2.7.26 after a
//! Sennet PKI reply. This is channel correlation, not an authenticated proof.
//! Unknown fields and request-like extensions are outside the observed profile.
use crate::protobuf::{Reader, Value, write_tag};
use alloc::vec::Vec;

pub const ROUTING_PORT: u32 = 5;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub request_id: u32,
    pub code: u32,
}
impl Response {
    pub const fn is_success(self) -> bool {
        self.code == 0
    }
    pub fn decode(data: &[u8]) -> Result<Self, RoutingError> {
        if data.len() > crate::transport::MAX_PAYLOAD_LEN {
            return Err(RoutingError::Malformed);
        }
        let mut reader = Reader::new(data);
        let (mut port, mut payload, mut request, mut trailing) = (None, None, None, None);
        for field in reader.by_ref() {
            match (field.number, field.value) {
                (1, Value::Varint(v)) => set(&mut port, v, 1)?,
                (2, Value::Len(v)) => set(&mut payload, v, 2)?,
                (6, Value::I32(v)) => set(&mut request, u32::from_le_bytes(v), 6)?,
                (9, Value::Varint(0)) => set(&mut trailing, (), 9)?,
                (1 | 2 | 6 | 9, _) => return Err(RoutingError::WireType(field.number)),
                _ => return Err(RoutingError::UnsupportedField(field.number)),
            }
        }
        if !reader.is_complete() {
            return Err(RoutingError::Malformed);
        }
        if port != Some(u64::from(ROUTING_PORT)) {
            return Err(RoutingError::NotRouting);
        }
        let request_id = request
            .filter(|id| *id != 0)
            .ok_or(RoutingError::MissingCorrelation)?;
        let mut inner = Reader::new(payload.ok_or(RoutingError::MissingBody)?);
        let mut code = None;
        for field in inner.by_ref() {
            match (field.number, field.value) {
                (3, Value::Varint(v)) => set(
                    &mut code,
                    u32::try_from(v).map_err(|_| RoutingError::CodeOverflow)?,
                    3,
                )?,
                (3, _) => return Err(RoutingError::WireType(3)),
                _ => return Err(RoutingError::UnsupportedField(field.number)),
            }
        }
        if !inner.is_complete() {
            return Err(RoutingError::Malformed);
        }
        Ok(Self {
            request_id,
            code: code.ok_or(RoutingError::MissingBody)?,
        })
    }
}
fn set<T>(slot: &mut Option<T>, value: T, field: u32) -> Result<(), RoutingError> {
    if slot.is_some() {
        return Err(RoutingError::Duplicate(field));
    }
    *slot = Some(value);
    Ok(())
}

/// Reproduce the observed success response for a nonzero request ID. The
/// transport owner supplies a fresh packet ID, addresses and channel cipher.
pub fn encode_ack(request_id: u32) -> Result<Vec<u8>, RoutingError> {
    if request_id == 0 {
        return Err(RoutingError::MissingCorrelation);
    }
    let mut out = alloc::vec![0x08, 0x05, 0x12, 0x02, 0x18, 0x00];
    write_tag(6, 5, &mut out);
    out.extend_from_slice(&request_id.to_le_bytes());
    out.extend_from_slice(&[0x48, 0]);
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingError {
    Malformed,
    NotRouting,
    MissingCorrelation,
    MissingBody,
    CodeOverflow,
    Duplicate(u32),
    WireType(u32),
    UnsupportedField(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_matches_the_identified_stock_capture() {
        let bytes = [0x08, 0x05, 0x12, 0x02, 0x18, 0, 0x35, 1, 0, 0, 0, 0x48, 0];
        assert_eq!(
            Response::decode(&bytes).unwrap(),
            Response {
                request_id: 1,
                code: 0
            }
        );
        assert_eq!(encode_ack(1).unwrap(), bytes);
        let mut nonzero = bytes;
        nonzero[5] = 7;
        assert!(!Response::decode(&nonzero).unwrap().is_success());
    }
    #[test]
    fn unsupported_requests_ambiguous_fields_and_partial_responses_are_refused() {
        let good = encode_ack(1).unwrap();
        for extra in [&[0x18, 1][..], &[0x35, 2, 0, 0, 0], &[0x48, 0], &[0x0a, 0]] {
            let mut bytes = good.clone();
            bytes.extend_from_slice(extra);
            assert!(Response::decode(&bytes).is_err());
        }
        for end in 0..good.len() - 2 {
            assert!(Response::decode(&good[..end]).is_err());
        }
        let mut missing = good.clone();
        missing[7..11].fill(0);
        assert_eq!(
            Response::decode(&missing),
            Err(RoutingError::MissingCorrelation)
        );
        assert!(encode_ack(0).is_err());
        let mut wrong = good;
        wrong[1] = 1;
        assert_eq!(Response::decode(&wrong), Err(RoutingError::NotRouting));
    }
}
