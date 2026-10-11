// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::{
    MAX_ESI, MAX_SYMBOLS_PER_BLOCK, MAX_TRANSFER_LENGTH, PROFILE_MAX_BLOCK_SYMBOLS,
    PROFILE_MAX_ESI, PROFILE_MAX_OBJECT, PROFILE_MAX_SYMBOL, require,
};
use raptorq::{EncodingPacket, ObjectTransmissionInformation, PayloadId};
use serde_json::{Value, json};

/// Validate integer range before calling the assertion-based crate constructor.
pub fn checked_payload_id(sbn: u8, esi: u32) -> Result<PayloadId, String> {
    require(esi <= MAX_ESI, "ESI exceeds 24 bits")?;
    Ok(PayloadId::new(sbn, esi))
}

pub fn parse_payload_id(bytes: &[u8]) -> Result<PayloadId, String> {
    let array: &[u8; 4] = bytes
        .try_into()
        .map_err(|_| "payload ID must be exactly four octets")?;
    Ok(PayloadId::deserialize(array))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Oti {
    pub transfer_length: u64,
    pub symbol_size: u16,
    pub source_blocks: u8,
    pub sub_blocks: u16,
    pub alignment: u8,
}

impl Oti {
    /// RFC field/range constraints, including Verified erratum 5548.
    /// This permits F=0; the executable composition profile rejects empty objects.
    pub fn validate(self) -> Result<Self, String> {
        require(
            self.transfer_length <= MAX_TRANSFER_LENGTH,
            "transfer length exceeds corrected RFC maximum",
        )?;
        require(
            self.symbol_size > 0
                && self.source_blocks > 0
                && self.sub_blocks > 0
                && self.alignment > 0,
            "T, Z, N and Al must be positive",
        )?;
        require(
            self.symbol_size.is_multiple_of(u16::from(self.alignment)),
            "T must be a multiple of Al",
        )?;
        let symbols = self.transfer_length.div_ceil(u64::from(self.symbol_size));
        require(
            symbols.div_ceil(u64::from(self.source_blocks)) <= MAX_SYMBOLS_PER_BLOCK,
            "source block exceeds K'_max",
        )?;
        Ok(self)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let bytes: &[u8; 12] = bytes
            .try_into()
            .map_err(|_| "OTI must be exactly twelve octets")?;
        let f = u64::from_be_bytes([0, 0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]]);
        // Reserved byte is not used to derive decoder state.
        Self {
            transfer_length: f,
            symbol_size: u16::from_be_bytes([bytes[6], bytes[7]]),
            source_blocks: bytes[8],
            sub_blocks: u16::from_be_bytes([bytes[9], bytes[10]]),
            alignment: bytes[11],
        }
        .validate()
    }

    pub fn to_crate(self) -> Result<ObjectTransmissionInformation, String> {
        self.validate()?;
        Ok(ObjectTransmissionInformation::new(
            self.transfer_length,
            self.symbol_size,
            self.source_blocks,
            self.sub_blocks,
            self.alignment,
        ))
    }

    /// Additional harness limits are profile restrictions, not RFC MUSTs.
    pub fn bounded(self) -> Result<Self, String> {
        self.bounded_for_alignment(4)
    }

    /// Independent peers may agree a distinct, explicit alignment profile.
    pub(crate) fn bounded_for_alignment(self, alignment: u8) -> Result<Self, String> {
        self.validate()?;
        require(
            (1..=PROFILE_MAX_OBJECT).contains(&self.transfer_length),
            "profile requires a nonempty object <=64KiB",
        )?;
        require(
            self.symbol_size <= PROFILE_MAX_SYMBOL,
            "profile symbol budget exceeded",
        )?;
        require(
            self.sub_blocks == 1 && self.alignment == alignment,
            "profile N/alignment differs from negotiated capability",
        )?;
        require(
            self.source_blocks <= 16,
            "profile source-block count exceeded",
        )?;
        require(
            u64::from(self.source_blocks)
                <= self.transfer_length.div_ceil(u64::from(self.symbol_size)),
            "profile rejects empty source blocks",
        )?;
        require(
            self.transfer_length
                .div_ceil(u64::from(self.symbol_size))
                .div_ceil(u64::from(self.source_blocks))
                <= PROFILE_MAX_BLOCK_SYMBOLS,
            "profile source-block work budget exceeded",
        )?;
        Ok(self)
    }
}

/// One-full-symbol bounded profile. RFC6330 also requires multiple-symbol
/// reception and permits shortened final source-symbol padding; those are unimplemented.
pub fn checked_packet(bytes: &[u8], oti: Oti) -> Result<EncodingPacket, String> {
    checked_packet_for_alignment(bytes, oti, 4)
}

pub(crate) fn checked_packet_for_alignment(
    bytes: &[u8],
    oti: Oti,
    alignment: u8,
) -> Result<EncodingPacket, String> {
    oti.bounded_for_alignment(alignment)?;
    require(
        bytes.len() == 4 + usize::from(oti.symbol_size),
        "profile packet must contain one full symbol",
    )?;
    let id = parse_payload_id(&bytes[..4])?;
    require(
        id.source_block_number() < oti.source_blocks,
        "SBN outside configured source blocks",
    )?;
    require(
        id.encoding_symbol_id() <= PROFILE_MAX_ESI,
        "profile ESI budget exceeded",
    )?;
    // All length and decoder-index preconditions are checked before crate deserialization.
    Ok(EncodingPacket::deserialize(bytes))
}

pub(crate) fn payload_id_cases() -> Result<Value, String> {
    for (sbn, esi, expected) in [
        (0, 0, [0, 0, 0, 0]),
        (7, 0x123456, [7, 0x12, 0x34, 0x56]),
        (255, MAX_ESI, [255, 255, 255, 255]),
    ] {
        let id = checked_payload_id(sbn, esi)?;
        require(id.serialize() == expected, "payload ID byte order")?;
        let parsed = parse_payload_id(&expected)?;
        require(
            parsed.source_block_number() == sbn && parsed.encoding_symbol_id() == esi,
            "payload ID fields",
        )?;
    }
    require(
        checked_payload_id(0, MAX_ESI + 1).is_err(),
        "oversized ESI accepted",
    )?;
    for length in [0, 1, 3, 5, 8] {
        require(
            parse_payload_id(&vec![0; length]).is_err(),
            "wrong payload ID length accepted",
        )?;
    }
    Ok(
        json!({"positive_boundaries": 3, "rejected_lengths": [0, 1, 3, 5, 8], "maximum_esi": MAX_ESI, "integer_overflow_rejected": true}),
    )
}

pub(crate) fn small_oti() -> Oti {
    Oti {
        transfer_length: 4097,
        symbol_size: 128,
        source_blocks: 2,
        sub_blocks: 1,
        alignment: 4,
    }
}

pub(crate) fn oti_cases() -> Result<Value, String> {
    let oti = small_oti();
    let bytes = [0, 0, 0, 0x10, 1, 0, 0, 128, 2, 0, 1, 4];
    require(
        oti.to_crate()?.serialize() == bytes,
        "OTI byte order/field widths",
    )?;
    require(Oti::parse(&bytes)? == oti, "OTI parsed fields")?;
    for length in [0, 8, 11, 13, 20] {
        require(
            Oti::parse(&vec![0; length]).is_err(),
            "wrong OTI length accepted",
        )?;
    }
    let max = Oti {
        transfer_length: MAX_TRANSFER_LENGTH,
        symbol_size: u16::MAX,
        source_blocks: u8::MAX,
        sub_blocks: 1,
        alignment: 1,
    };
    max.validate()?;
    require(
        Oti::parse(&max.to_crate()?.serialize())? == max,
        "corrected maximum round trip",
    )?;
    for invalid in [
        Oti {
            transfer_length: MAX_TRANSFER_LENGTH + 1,
            ..max
        },
        Oti {
            transfer_length: 946_270_874_880,
            ..max
        },
        Oti {
            symbol_size: 0,
            ..oti
        },
        Oti {
            source_blocks: 0,
            ..oti
        },
        Oti {
            sub_blocks: 0,
            ..oti
        },
        Oti {
            alignment: 0,
            ..oti
        },
        Oti {
            symbol_size: 127,
            ..oti
        },
        Oti {
            transfer_length: (MAX_SYMBOLS_PER_BLOCK + 1) * 128,
            source_blocks: 1,
            ..oti
        },
    ] {
        require(invalid.validate().is_err(), "invalid OTI accepted")?;
    }
    Oti {
        transfer_length: 0,
        ..oti
    }
    .validate()?;
    require(
        Oti {
            transfer_length: 0,
            ..oti
        }
        .bounded()
        .is_err(),
        "empty object profile restriction missing",
    )?;
    Ok(
        json!({"oti_octets": 12, "positive_fields": true, "corrected_max_transfer_length": MAX_TRANSFER_LENGTH, "verified_erratum": 5548, "invalid_parameter_cases": 8, "rejected_lengths": [0, 8, 11, 13, 20], "rfc_allows_zero_transfer_length": true}),
    )
}

pub(crate) fn packet_preflight_cases() -> Result<Value, String> {
    let oti = small_oti();
    let mut packet = vec![0; 4 + usize::from(oti.symbol_size)];
    checked_packet(&packet, oti)?;
    for length in [0, 3, 4, 131, 133, 260] {
        require(
            checked_packet(&vec![0; length], oti).is_err(),
            "bad profile packet length accepted",
        )?;
    }
    packet[0] = oti.source_blocks;
    require(
        checked_packet(&packet, oti).is_err(),
        "out-of-range SBN accepted",
    )?;
    packet[..4].copy_from_slice(&checked_payload_id(0, PROFILE_MAX_ESI + 1)?.serialize());
    require(
        checked_packet(&packet, oti).is_err(),
        "profile ESI budget missing",
    )?;
    require(
        Oti {
            transfer_length: PROFILE_MAX_OBJECT + 1,
            ..oti
        }
        .bounded()
        .is_err(),
        "object budget missing",
    )?;
    require(
        Oti {
            transfer_length: PROFILE_MAX_OBJECT,
            symbol_size: 4,
            source_blocks: 1,
            ..oti
        }
        .bounded()
        .is_err(),
        "source-block work budget missing",
    )?;
    Ok(
        json!({"exact_symbol_octets": 128, "truncated_and_multi_symbol_profile_packets_rejected": true, "sbn_checked_before_decoder": true, "esi_and_object_budgets_checked": true, "source_symbols_per_block_budget": PROFILE_MAX_BLOCK_SYMBOLS, "oversized_source_block_rejected_before_decoder": true, "complete_rfc_packet_receiver": false}),
    )
}
