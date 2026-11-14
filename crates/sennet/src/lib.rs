#![forbid(unsafe_code)]
#![no_std]

//! Sennet: the schema-free protobuf wire reader Signalman's stock-companion
//! client uses to talk to a Meshtastic-compatible radio's client stream.
//!
//! A stock radio holds its keys and runs the mesh. Since 2026-11-14 this crate
//! carries no transport, cipher, routing or packet-ID allocator of its own
//! (Mark: "Stock radios only").
//!
//! A sennet is a ceremonial fanfare for a procession.
//!
//! # Provenance
//!
//! Sennet is a clean-room implementation. Every byte format here comes from one of three
//! sources only: publicly documented wire and frame layouts, Google's public protobuf wire
//! standard, or direct observation of bytes a device emits. No third-party protocol source,
//! schema definition, or client library was consulted. See `PROVENANCE.md`.

extern crate alloc;

pub mod protobuf;

/// Largest text payload a Meshtastic-compatible radio carries in one packet: the
/// 237-byte radio payload less five bytes of text-port envelope.
pub const MAX_APPLICATION_PAYLOAD: usize = 237 - 5;
