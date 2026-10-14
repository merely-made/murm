//! Stock Stream API client fields; schema source is recorded in the R4 receipt.
//! This host client is separate from Sennet's independently authored RF codec.
use super::frame::invalid;
use sennet::protobuf::{Reader, Value, write_tag, write_varint};
use std::io;

pub(super) fn fields(bytes: &[u8]) -> io::Result<Vec<sennet::protobuf::Field<'_>>> {
    let mut reader = Reader::new(bytes);
    let fields: Vec<_> = reader.by_ref().collect();
    if !reader.is_complete() {
        return Err(invalid("malformed companion protobuf"));
    }
    // Reject duplicate known/singular fields instead of accepting conflicting identities.
    let mut seen = std::collections::HashSet::new();
    if fields.iter().any(|f| !seen.insert(f.number)) {
        return Err(invalid("duplicate companion protobuf field"));
    }
    Ok(fields)
}
pub(super) fn uint(bytes: &[u8], number: u32) -> io::Result<u32> {
    for f in fields(bytes)? {
        if f.number == number {
            return match f.value {
                Value::Varint(v) => {
                    u32::try_from(v).map_err(|_| invalid("companion integer overflow"))
                }
                Value::I32(v) => Ok(u32::from_le_bytes(v)),
                _ => Err(invalid("wrong companion integer wire type")),
            };
        }
    }
    Ok(0)
}
pub(super) fn signed(bytes: &[u8], number: u32) -> io::Result<i32> {
    for field in fields(bytes)? {
        if field.number == number {
            return match field.value {
                Value::Varint(value)
                    if value <= i32::MAX as u64 || value >= u64::MAX - i32::MAX as u64 =>
                {
                    Ok(value as u32 as i32)
                }
                _ => Err(invalid("invalid signed companion integer")),
            };
        }
    }
    Ok(0)
}
pub(super) fn nested(bytes: &[u8], number: u32) -> io::Result<Option<&[u8]>> {
    for f in fields(bytes)? {
        if f.number == number {
            return match f.value {
                Value::Len(v) => Ok(Some(v)),
                _ => Err(invalid("wrong companion message wire type")),
            };
        }
    }
    Ok(None)
}
pub(super) fn var(out: &mut Vec<u8>, n: u32, v: u32) {
    write_tag(n, 0, out);
    write_varint(u64::from(v), out);
}
fn fixed(out: &mut Vec<u8>, n: u32, v: u32) {
    write_tag(n, 5, out);
    out.extend_from_slice(&v.to_le_bytes());
}
pub(super) fn bytes(out: &mut Vec<u8>, n: u32, v: &[u8]) {
    write_tag(n, 2, out);
    write_varint(v.len() as u64, out);
    out.extend_from_slice(v);
}
pub(super) fn configure(nonce: u32) -> Vec<u8> {
    let mut v = vec![];
    var(&mut v, 3, nonce);
    v
}
pub(super) fn text(destination: u32, channel: u8, id: u32, text: &str) -> Vec<u8> {
    let mut data = vec![];
    var(&mut data, 1, 1);
    bytes(&mut data, 2, text.as_bytes());
    let mut packet = vec![];
    fixed(&mut packet, 2, destination);
    var(&mut packet, 3, u32::from(channel));
    bytes(&mut packet, 4, &data);
    fixed(&mut packet, 6, id);
    var(&mut packet, 9, 3);
    var(&mut packet, 10, u32::from(destination != u32::MAX));
    let mut to_radio = vec![];
    bytes(&mut to_radio, 1, &packet);
    to_radio
}
