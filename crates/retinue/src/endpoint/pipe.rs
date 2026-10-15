//! PipeInterface: HDLC over a child process's stdio, respawned when it exits
//! (`PipeInterface.py` 41-207).
//!
//! Seam stub: the settings and the attach call. Unit V4 (C4) implements them.

use alloc::vec::Vec;

use std::ffi::OsString;
use std::io;
use std::time::Duration;

use crate::ifac::Ifac;

use super::interface::InterfaceId;
use super::runtime::Endpoint;

/// The pipe's hardware MTU (`PipeInterface.py` 76).
pub const PIPE_HW_MTU: usize = 1064;

/// A PipeInterface: RNS's `command` and `respawn_delay`.
#[derive(Clone, Debug)]
pub struct PipeConfig {
    /// The program and its arguments, already split; RNS splits `command` with `shlex`.
    pub argv: Vec<OsString>,
    /// Wait before respawning a child that exited (`PipeInterface.py` 59, 175-190).
    pub respawn_delay: Duration,
    /// Access code; a stock pipe without `ifac_size` uses [`Ifac::for_serial`]'s 8 bytes.
    pub ifac: Option<Ifac>,
}

impl PipeConfig {
    /// RNS's defaults: respawn after 5 s, no access code.
    pub fn new(argv: Vec<OsString>) -> Self {
        Self {
            argv,
            respawn_delay: Duration::from_secs(5),
            ifac: None,
        }
    }
}

impl Endpoint {
    /// Spawn `config.argv` and carry an interface over its stdin and stdout, respawning it
    /// under the same id whenever it exits.
    pub async fn attach_pipe(&self, config: PipeConfig) -> io::Result<InterfaceId> {
        let _ = config;
        Err(io::ErrorKind::Unsupported.into())
    }
}
