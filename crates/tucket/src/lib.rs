#![forbid(unsafe_code)]
#![no_std]

//! Tucket: the MeshCore companion frame types Signalman's stock-companion
//! client reads.
//!
//! A stock MeshCore radio holds its keys and runs the mesh; the host speaks the
//! public companion serial API to it. Since 2026-11-14 this crate carries no
//! MeshCore node, cipher or identity of its own (Mark: "Stock radios only").
//!
//! A tucket is a trumpet flourish announcing a single arrival.

pub mod companion;
