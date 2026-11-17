//! Radio-free, carrier-neutral wall-node control contract.
//!
//! The semantic request lives inside `retinue::command::Command`; the outer command retains
//! target, signer, counter, signature, and verification authority. The grammar, replies,
//! capability facts, and durable journal are `seneschal::control`, re-exported here so board
//! code keeps one path. What stays in this crate is board-side: the async runtime that orders
//! journal writes against flash and radio, and the WN0 volatile admission seam.

mod admission;
mod runtime;
#[cfg(test)]
mod test_authority;

pub use seneschal::control::*;

pub use admission::{Admission, RequestAdmission};
pub use runtime::{
    BootState, ConfigApplier, ControlRuntime, DurableScratch, DurableScratchError, LiveOutcome,
    MAX_PROVISIONAL_LIFETIME_MS, MIN_DURABLE_SLOT_BYTES, MIN_PROVISIONAL_LIFETIME_MS,
    PreparedCommit, PreparedProvisional, QuietExit, QuietGuard, QuietWindow, RuntimeError,
};
