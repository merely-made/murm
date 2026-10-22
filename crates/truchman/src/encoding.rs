//! I2P's two text encodings: base64 with the `-~` alphabet, and lowercase unpadded base32
//! for `.b32.i2p` names.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-~";
const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Encode with I2P's base64 alphabet, padded with `=`.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match *chunk {
            [a, b, c] => u32::from_be_bytes([0, a, b, c]),
            [a, b] => u32::from_be_bytes([0, a, b, 0]),
            [a] => u32::from_be_bytes([0, a, 0, 0]),
            _ => unreachable!(),
        };
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decode I2P base64, padded or not. `None` on a character outside the alphabet, padding
/// anywhere but the end, or an impossible length.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let body = text.trim_end_matches('=');
    if text.len() - body.len() > 2 || body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in body.bytes() {
        let v = B64.iter().position(|&a| a == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Leftover bits must be zero, or the text was not produced by an encoder.
    (acc == 0).then_some(out)
}

/// Encode as RFC 4648 base32, lowercase and unpadded, as `.b32.i2p` names are.
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u64, 0u32);
    for &b in bytes {
        acc = acc << 8 | u64::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[(acc >> bits & 0x1f) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(B32[(acc << (5 - bits) & 0x1f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors_in_the_i2p_alphabet() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).unwrap(), plain.as_bytes());
            assert_eq!(
                base64_decode(coded.trim_end_matches('=')).unwrap(),
                plain.as_bytes()
            );
        }
        // 0xfb 0xff encodes to the two characters that differ from the standard alphabet.
        assert_eq!(base64_encode(&[0xfb, 0xff, 0xbf]), "-~-~");
        assert_eq!(base64_decode("-~-~").unwrap(), [0xfb, 0xff, 0xbf]);
    }

    #[test]
    fn base64_refuses_foreign_characters_and_bad_padding() {
        assert!(base64_decode("Zm9v+A==").is_none());
        assert!(base64_decode("Zm9v/A==").is_none());
        assert!(base64_decode("Zg=a").is_none());
        assert!(base64_decode("Zg===").is_none());
        assert!(base64_decode("Z").is_none());
        // Non-zero trailing bits.
        assert!(base64_decode("Zh==").is_none());
    }

    #[test]
    fn base32_matches_rfc4648_vectors_lowercase_unpadded() {
        for (plain, coded) in [
            ("", ""),
            ("f", "my"),
            ("fo", "mzxq"),
            ("foo", "mzxw6"),
            ("foob", "mzxw6yq"),
            ("fooba", "mzxw6ytb"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(base32_encode(plain.as_bytes()), coded);
        }
        assert_eq!(base32_encode(&[0u8; 32]).len(), 52);
    }
}
