// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Process adapter for independent encoder/decoder campaigns.
//! RQI1/RQO1 is a harness protocol, never an RFC6330 content-delivery transport.

use super::{Oti, PROFILE_MAX_ESI, checked_packet_for_alignment, parse_payload_id, require};
use raptorq::{Decoder, Encoder, EncodingPacket, PayloadId};

pub const MAX_REQUEST_BYTES: usize = 512 * 1024;
const MAX_PACKETS: usize = 1024;

struct Input<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Input<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or("request length overflow")?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or("truncated interop request")?;
        self.offset = end;
        Ok(result)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes: [u8; 2] = self.take(2)?.try_into().map_err(|_| "u16 length")?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| "u32 length")?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn complete(&self) -> Result<(), String> {
        require(
            self.offset == self.bytes.len(),
            "trailing interop request bytes",
        )
    }
}

fn alignment_profile(oti: Oti) -> Result<u8, String> {
    require(
        matches!(oti.alignment, 1 | 4),
        "interop supports only explicit Al=1 or Al=4",
    )?;
    oti.bounded_for_alignment(oti.alignment)?;
    Ok(oti.alignment)
}

pub fn run_request(request: &[u8]) -> Result<Vec<u8>, String> {
    require(
        request.len() <= MAX_REQUEST_BYTES,
        "interop request budget exceeded",
    )?;
    let mut input = Input {
        bytes: request,
        offset: 0,
    };
    require(input.take(4)? == b"RQI1", "unknown interop request version")?;
    let command = input.take(1)?[0];
    let oti = Oti::parse(input.take(12)?)?;
    let alignment = alignment_profile(oti)?;
    let mut output = b"RQO1".to_vec();
    output.push(command);
    match command {
        b'E' => {
            let length = input.u32()? as usize;
            require(
                length as u64 == oti.transfer_length,
                "object length differs from OTI",
            )?;
            let object = input.take(length)?;
            let count = usize::from(input.u16()?);
            require(count <= MAX_PACKETS, "interop packet count budget exceeded")?;
            let mut ids = Vec::with_capacity(count);
            for _ in 0..count {
                let id = parse_payload_id(input.take(4)?)?;
                require(
                    id.source_block_number() < oti.source_blocks,
                    "requested SBN outside OTI",
                )?;
                require(
                    id.encoding_symbol_id() <= PROFILE_MAX_ESI,
                    "requested ESI budget exceeded",
                )?;
                ids.push(id);
            }
            input.complete()?;
            // Allocation and assertion-based codec entry happens after all request checks.
            let encoder = Encoder::new(object, oti.to_crate()?);
            let packets = encode_ids(&encoder, &ids)?;
            output.extend_from_slice(&(packets.len() as u16).to_be_bytes());
            for packet in packets {
                let bytes = packet.serialize();
                checked_packet_for_alignment(&bytes, oti, alignment)?;
                output.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
                output.extend_from_slice(&bytes);
            }
        }
        b'D' => {
            let count = usize::from(input.u16()?);
            require(count <= MAX_PACKETS, "interop packet count budget exceeded")?;
            let mut packets = Vec::with_capacity(count);
            for _ in 0..count {
                let length = usize::from(input.u16()?);
                packets.push(checked_packet_for_alignment(
                    input.take(length)?,
                    oti,
                    alignment,
                )?);
            }
            input.complete()?;
            let mut decoder = Decoder::new(oti.to_crate()?);
            let mut result = None;
            for packet in packets {
                if let Some(decoded) = decoder.decode(packet) {
                    result = Some(decoded);
                }
            }
            if let Some(object) = result {
                require(
                    object.len() as u64 == oti.transfer_length,
                    "decoded object length differs from OTI",
                )?;
                output.push(1);
                output.extend_from_slice(&(object.len() as u32).to_be_bytes());
                output.extend_from_slice(&object);
            } else {
                output.push(0);
            }
        }
        _ => return Err("unsupported interop command".into()),
    }
    Ok(output)
}

pub(crate) fn encode_ids(
    encoder: &Encoder,
    ids: &[PayloadId],
) -> Result<Vec<EncodingPacket>, String> {
    let mut packets = Vec::with_capacity(ids.len());
    for id in ids {
        let block = encoder
            .get_block_encoders()
            .get(usize::from(id.source_block_number()))
            .ok_or("requested source block does not exist")?;
        let source = block.source_packets();
        let esi = id.encoding_symbol_id();
        let packet = if (esi as usize) < source.len() {
            source
                .get(esi as usize)
                .ok_or("requested source symbol does not exist")?
                .clone()
        } else {
            block
                .repair_packets(esi - source.len() as u32, 1)
                .into_iter()
                .next()
                .ok_or("repair generation returned no packet")?
        };
        packets.push(packet);
    }
    Ok(packets)
}

#[cfg(test)]
mod tests {
    use super::run_request;

    #[test]
    fn malformed_requests_are_refused_before_codec() {
        let mut valid = b"RQI1E".to_vec();
        valid.extend_from_slice(&super::super::small_oti().to_crate().unwrap().serialize());
        valid.extend_from_slice(&4097u32.to_be_bytes());
        valid.extend_from_slice(&vec![0; 4097]);
        valid.extend_from_slice(&1u16.to_be_bytes());
        valid.extend_from_slice(&[0, 0, 0, 0]);
        for length in [0, 4, 5, 16, 20, valid.len() - 1] {
            assert!(run_request(&valid[..length]).is_err());
        }
        let mut invalid = valid.clone();
        invalid[0] = 0;
        assert!(run_request(&invalid).is_err());
        invalid = valid;
        let sbn = invalid.len() - 4;
        invalid[sbn] = 2;
        assert!(run_request(&invalid).is_err());
    }
}
