// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The byte-level primitives every wire form in this crate shares.
//!
//! Private. Three things live here and nothing else: length-prefixed field
//! encoding, hex, and a constant-time byte comparison. They are gathered so
//! that the transcript, the frame, and the identifier modules cannot drift
//! into three slightly different notions of "the same bytes".

/// Compares two byte slices without an early exit on the first difference.
///
/// Unequal lengths short-circuit — a length is not secret here, and the
/// identifiers this guards are all fixed width anyway. The fold itself runs
/// over every byte, and `black_box` keeps the optimizer from reintroducing
/// the branch it was written to avoid.
#[inline]
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    core::hint::black_box(acc) == 0
}

/// Appends `field` to `out` behind a four-byte big-endian length.
///
/// Every field in a canonical encoding goes through here, fixed-width ones
/// included. Prefixing a field whose width is already known is redundant on
/// its own terms, and it is exactly what makes the whole encoding injective:
/// with no unprefixed run anywhere, no regrouping of the same bytes across
/// two fields can produce an identical transcript.
///
/// Callers bound their fields before reaching this; the cast is safe because
/// nothing in this crate encodes a field near `u32::MAX`.
pub(crate) fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_le_bytes());
    out.extend_from_slice(field);
}

/// Lowercase hex, no separators.
pub(crate) fn to_hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Uppercase hex, no separators.
pub(crate) fn to_hex_upper(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// One hex digit's value, either case.
pub(crate) fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// One hex digit's value, uppercase only (RFC 8122's `UHEX`).
pub(crate) fn hex_digit_upper(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// RFC 4648 §5 base64url alphabet: `A-Z`, `a-z`, `0-9`, `-`, `_`.
///
/// Index into this table is the encoder's whole job; [`b64url_decode`] below
/// is this table inverted.
const B64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes `bytes` as base64url (RFC 4648 §5) with no padding.
///
/// Hand-rolled rather than pulled in as a dependency: the core's whole
/// argument for staying `blake3`-plus-`thiserror` is that a wire primitive
/// this small does not earn a crate. Unpadded, because the padding character
/// `=` has no job to do here — [`InviteV1::decode`](crate::InviteV1::decode)
/// gets the length from the surrounding fragment text, not from `=` runs, and
/// an unpadded fragment is shorter in a URL besides.
pub(crate) fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(B64URL_ALPHABET[usize::try_from(n >> 18 & 0x3f).unwrap()] as char);
        out.push(B64URL_ALPHABET[usize::try_from(n >> 12 & 0x3f).unwrap()] as char);
        out.push(B64URL_ALPHABET[usize::try_from(n >> 6 & 0x3f).unwrap()] as char);
        out.push(B64URL_ALPHABET[usize::try_from(n & 0x3f).unwrap()] as char);
    }
    match chunks.remainder() {
        [] => {},
        &[b0] => {
            let n = u32::from(b0) << 16;
            out.push(B64URL_ALPHABET[usize::try_from(n >> 18 & 0x3f).unwrap()] as char);
            out.push(B64URL_ALPHABET[usize::try_from(n >> 12 & 0x3f).unwrap()] as char);
        },
        &[b0, b1] => {
            let n = (u32::from(b0) << 16) | (u32::from(b1) << 8);
            out.push(B64URL_ALPHABET[usize::try_from(n >> 18 & 0x3f).unwrap()] as char);
            out.push(B64URL_ALPHABET[usize::try_from(n >> 12 & 0x3f).unwrap()] as char);
            out.push(B64URL_ALPHABET[usize::try_from(n >> 6 & 0x3f).unwrap()] as char);
        },
        _ => unreachable!("chunks_exact(3)'s remainder is under 3 bytes"),
    }
    out
}

/// One base64url character's 6-bit value, or `None` outside the alphabet.
///
/// `=` is not in the alphabet, so a padded string is already rejected here —
/// [`b64url_decode`]'s explicit padding check exists so that rejection reads
/// as a named rule rather than a side effect of the lookup failing.
fn b64url_digit(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// Decodes unpadded base64url (RFC 4648 §5), strictly.
///
/// Three ways in to reject, all before any byte is produced: a padding
/// character anywhere, a character outside the 64-symbol alphabet, or a
/// length making `len % 4 == 1` — impossible for any unpadded encoding,
/// since one leftover base64 character carries only 6 bits and cannot
/// resolve to a whole byte. Nothing here truncates a bad tail or guesses at
/// intent; a string that does not decode exactly returns `None`.
pub(crate) fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    if bytes.contains(&b'=') {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    let mut chunks = bytes.chunks_exact(4);
    for chunk in &mut chunks {
        let a = b64url_digit(chunk[0])?;
        let b = b64url_digit(chunk[1])?;
        let c = b64url_digit(chunk[2])?;
        let d = b64url_digit(chunk[3])?;
        let n = (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6) | u32::from(d);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }
    match chunks.remainder() {
        [] => {},
        &[c0, c1] => {
            let a = b64url_digit(c0)?;
            let b = b64url_digit(c1)?;
            let n = (u32::from(a) << 18) | (u32::from(b) << 12);
            out.push((n >> 16) as u8);
        },
        &[c0, c1, c2] => {
            let a = b64url_digit(c0)?;
            let b = b64url_digit(c1)?;
            let c = b64url_digit(c2)?;
            let n = (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6);
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        },
        _ => return None, // len % 4 == 1 is already caught above.
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_agrees_with_ordinary_equality() {
        assert!(ct_eq(b"", b""));
        assert!(ct_eq(&[1, 2, 3], &[1, 2, 3]));
        assert!(!ct_eq(&[1, 2, 3], &[1, 2, 4]));
        assert!(!ct_eq(&[9, 2, 3], &[1, 2, 3]));
        assert!(!ct_eq(&[1, 2, 3], &[1, 2]));
    }

    #[test]
    fn hex_round_trips_in_both_cases() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        assert_eq!(to_hex_lower(&bytes), "000fa5ff");
        assert_eq!(to_hex_upper(&bytes), "000FA5FF");
        assert_eq!(hex_digit(b'f'), Some(15));
        assert_eq!(hex_digit(b'F'), Some(15));
        assert_eq!(hex_digit(b'g'), None);
        assert_eq!(hex_digit_upper(b'f'), None);
        assert_eq!(hex_digit_upper(b'F'), Some(15));
    }

    #[test]
    fn push_field_writes_a_little_endian_length() {
        let mut out = Vec::new();
        push_field(&mut out, b"ab");
        // u64 little-endian, the same prefix shape
        // `graphshell::browser_carrier` writes.
        assert_eq!(out, vec![2, 0, 0, 0, 0, 0, 0, 0, b'a', b'b']);
    }

    /// RFC 4648's own test vectors (`""`, `"f"`, `"fo"`, ... `"foobar"`),
    /// stripped of `=` padding — an independent oracle rather than a value
    /// this same encoder produced, so the test can fail.
    #[test]
    fn matches_the_rfc_4648_test_vectors_unpadded() {
        let vectors: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in vectors {
            let encoded = b64url_encode(input);
            assert_eq!(&encoded, expected, "encoding {input:?}");
            assert_eq!(
                b64url_decode(expected).as_deref(),
                Some(*input),
                "decoding {expected:?}"
            );
        }
    }

    /// A 32-byte value (the width every fixed field in this crate uses),
    /// checked against a value computed independently with Python's
    /// `base64.urlsafe_b64encode`, not merely round-tripped through this
    /// module's own two functions.
    #[test]
    fn a_32_byte_value_matches_an_independently_computed_encoding() {
        let bytes: Vec<u8> = (0u8..32).collect();
        let expected = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
        assert_eq!(b64url_encode(&bytes), expected);
        assert_eq!(b64url_decode(expected).as_deref(), Some(bytes.as_slice()));
    }

    #[test]
    fn decode_rejects_padding_invalid_alphabet_and_impossible_lengths() {
        // Padding, even where the length would otherwise be valid.
        assert_eq!(b64url_decode("Zg=="), None);
        assert_eq!(b64url_decode("AA=="), None);
        assert_eq!(b64url_decode("A==="), None);

        // Characters outside the url-safe alphabet: standard base64's `+`
        // and `/`, and an ordinary non-base64 character.
        assert_eq!(b64url_decode("Zm+8"), None);
        assert_eq!(b64url_decode("Zm/8"), None);
        assert_eq!(b64url_decode("Zg!!"), None);

        // `len % 4 == 1`: one leftover base64 character, which can never
        // resolve to a whole byte.
        assert_eq!(b64url_decode("A"), None);
        assert_eq!(b64url_decode("AAAAA"), None);
    }
}
