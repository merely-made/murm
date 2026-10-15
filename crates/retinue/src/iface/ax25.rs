//! The AX.25 UI header RNS's AX25KISSInterface puts before every packet
//! (`AX25KISSInterface.py` 60-65, 104-142, 268-348).
//!
//! Sixteen bytes: destination `APZRNS`-0, the station's callsign and SSID with the
//! address-extension bit, control UI (0x03) and PID "no layer 3" (0xF0). Callsigns are
//! padded with a shifted space (0x40); RNS pads with 0x20 (286-298), which [`strip`] still
//! accepts, so a 3-5 character callsign differs from stock on the air but interoperates.

use alloc::vec::Vec;

use super::kiss;

/// Header length (`AX25KISSInterface.py` 64).
pub const HEADER_LEN: usize = 16;
pub const CTRL_UI: u8 = 0x03;
pub const PID_NO_LAYER3: u8 = 0xF0;
/// The largest frame received: a KISS carrier's frame plus the header
/// (`AX25KISSInterface.py` 333).
pub const MAX_FRAME: usize = kiss::HW_MTU + HEADER_LEN;
/// The destination RNS addresses every frame to (`AX25KISSInterface.py` 119-120).
const DESTINATION: &[u8; 6] = b"APZRNS";

/// A station's precomputed header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ax25Header([u8; HEADER_LEN]);

/// A callsign or SSID RNS refuses (`AX25KISSInterface.py` 134-138).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ax25Error {
    /// Not 3 to 6 ASCII characters.
    Callsign,
    /// Above 15.
    Ssid,
}

impl core::fmt::Display for Ax25Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Callsign => f.write_str("AX.25 callsign must be 3 to 6 ASCII characters"),
            Self::Ssid => f.write_str("AX.25 SSID must be 0 to 15"),
        }
    }
}

impl Ax25Header {
    /// The header for `callsign` (uppercased, as RNS does) and `ssid`.
    pub fn new(callsign: &str, ssid: u8) -> Result<Self, Ax25Error> {
        if !callsign.is_ascii() || !(3..=6).contains(&callsign.len()) {
            return Err(Ax25Error::Callsign);
        }
        if ssid > 15 {
            return Err(Ax25Error::Ssid);
        }
        let mut header = [0; HEADER_LEN];
        address(&mut header[..7], DESTINATION, 0, false);
        address(&mut header[7..14], callsign.as_bytes(), ssid, true);
        header[14] = CTRL_UI;
        header[15] = PID_NO_LAYER3;
        Ok(Self(header))
    }

    pub fn bytes(&self) -> &[u8; HEADER_LEN] {
        &self.0
    }

    /// The header followed by `packet`.
    pub fn wrap(&self, packet: &[u8]) -> Vec<u8> {
        [&self.0[..], packet].concat()
    }
}

/// One 7-byte address field: six shifted characters, then `0x60 | ssid << 1 | last`.
fn address(out: &mut [u8], call: &[u8], ssid: u8, last: bool) {
    for (i, byte) in out[..6].iter_mut().enumerate() {
        *byte = call.get(i).map_or(b' ', u8::to_ascii_uppercase) << 1;
    }
    out[6] = 0x60 | ssid << 1 | u8::from(last);
}

/// The packet after a received UI header, or `None` for a frame that is not one: too short
/// to carry a packet, a digipeated path, or another control or PID. RNS strips 16 bytes
/// unchecked (`AX25KISSInterface.py` 268-271).
pub fn strip(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() <= HEADER_LEN {
        return None;
    }
    let (header, packet) = frame.split_at(HEADER_LEN);
    let chars_ok = header[..6].iter().chain(&header[7..13]).all(|b| b & 1 == 0);
    let ends_ok = header[6] & 1 == 0 && header[13] & 1 == 1;
    (chars_ok && ends_ok && header[14] == CTRL_UI && header[15] == PID_NO_LAYER3).then_some(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_character_callsigns_match_stock_bytes() {
        let header = Ax25Header::new("n0call", 7).unwrap();
        // RNS: `[c << 1 for c in b"APZRNS"] + [0x60]`, the source with `0x60 | 7 << 1 | 1`.
        let mut stock: Vec<u8> = b"APZRNS".iter().map(|c| c << 1).collect();
        stock.push(0x60);
        stock.extend(b"N0CALL".iter().map(|c| c << 1));
        stock.extend([0x60 | 7 << 1 | 1, 0x03, 0xF0]);
        assert_eq!(header.bytes().as_slice(), stock.as_slice());
        assert_eq!(strip(&header.wrap(b"pkt")), Some(&b"pkt"[..]));
    }

    #[test]
    fn short_callsigns_pad_with_a_shifted_space_and_accept_stock_padding() {
        let header = Ax25Header::new("ab1", 15).unwrap();
        assert_eq!(
            &header.bytes()[7..14],
            &[0x82, 0x84, 0x62, 0x40, 0x40, 0x40, 0x7F]
        );
        let mut stock = header.wrap(b"x");
        stock[10..13].fill(0x20);
        assert_eq!(strip(&stock), Some(&b"x"[..]));
    }

    #[test]
    fn invalid_settings_and_frames_are_refused() {
        assert_eq!(Ax25Header::new("ab", 0), Err(Ax25Error::Callsign));
        assert_eq!(Ax25Header::new("abcdefg", 0), Err(Ax25Error::Callsign));
        assert_eq!(Ax25Header::new("ñoño", 0), Err(Ax25Error::Callsign));
        assert_eq!(Ax25Header::new("n0call", 16), Err(Ax25Error::Ssid));
        let good = Ax25Header::new("n0call", 0).unwrap().wrap(b"p");
        assert_eq!(strip(&good[..HEADER_LEN]), None, "header only");
        for (at, value) in [(14, 0x13), (15, 0xCC), (13, 0x60), (6, 0x61), (0, 0x83)] {
            let mut bad = good.clone();
            bad[at] = value;
            assert_eq!(strip(&bad), None, "byte {at} = {value:#x}");
        }
    }
}
