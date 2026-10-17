use super::*;
use crate::store::SlotError;

pub(super) fn encode(
    sequence: u32,
    snapshot: &Snapshot,
    out: &mut [u8; SLOT_LEN],
) -> Result<usize, Fault> {
    let mut body = [0; SLOT_LEN - store::HEADER_LEN];
    body[..4].copy_from_slice(b"TKC0");
    body[4..6].copy_from_slice(&1_u16.to_le_bytes());
    body[6..8].copy_from_slice(&(snapshot.keys.len() as u16).to_le_bytes());
    body[8..40].copy_from_slice(&snapshot.scope.local_public);
    body[40..72].copy_from_slice(&snapshot.scope.profile_digest);
    for (i, key) in snapshot.keys.iter().enumerate() {
        body[72 + i * 32..104 + i * 32].copy_from_slice(key);
    }
    store::encode(sequence, &body[..72 + snapshot.keys.len() * 32], out)
        .map_err(|_| Fault::Capacity)
}

pub(super) fn decode(bytes: &[u8; SLOT_LEN]) -> Result<Option<(u32, Snapshot)>, Fault> {
    let record = match store::decode(bytes) {
        Ok(record) => record,
        Err(SlotError::Blank | SlotError::BadMagic | SlotError::BadCrc | SlotError::Truncated) => {
            return Ok(None);
        }
        Err(SlotError::UnsupportedVersion(_) | SlotError::BadLength) => {
            return Err(Fault::Unsupported);
        }
    };
    let b = record.body;
    if b.len() < 8 || &b[..4] != b"TKC0" || b[4..6] != 1_u16.to_le_bytes() {
        return Err(Fault::Unsupported);
    }
    let count = u16::from_le_bytes([b[6], b[7]]) as usize;
    if count > MAX_KEYS {
        return Err(Fault::Capacity);
    }
    if b.len() != 72 + count * 32 {
        return Err(Fault::Corrupt);
    }
    let scope = Scope {
        local_public: b[8..40].try_into().map_err(|_| Fault::Corrupt)?,
        profile_digest: b[40..72].try_into().map_err(|_| Fault::Corrupt)?,
    };
    let mut keys = Vec::<_, MAX_KEYS>::new();
    for key in b[72..].as_chunks::<32>().0 {
        keys.push(*key).map_err(|_| Fault::Capacity)?;
    }
    Ok(Some((record.sequence, Snapshot::new(scope, &keys)?)))
}

pub(super) fn select(
    a: &[u8; SLOT_LEN],
    b: &[u8; SLOT_LEN],
) -> Result<Option<(Slot, u32, Snapshot)>, Fault> {
    let da = decode(a)?;
    let db = decode(b)?;
    match (da, db) {
        // Erased commit words mean neither staged candidate ever became
        // authoritative, including a cut during the very first body program.
        (None, None) if a[..4].iter().chain(&b[..4]).all(|v| *v == 0xff) => Ok(None),
        (None, None) => Err(Fault::Corrupt),
        (Some((sa, ka)), Some((sb, kb))) if sa == sb && ka != kb => Err(Fault::Ambiguous),
        (Some((sa, _)), Some((sb, kb))) if sb > sa => Ok(Some((Slot::B, sb, kb))),
        (Some((sa, ka)), _) => Ok(Some((Slot::A, sa, ka))),
        (_, Some((sb, kb))) => Ok(Some((Slot::B, sb, kb))),
    }
}
