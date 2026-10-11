//! PipeInterface: HDLC over a child process's stdio, respawned when it exits
//! (`PipeInterface.py` 41-207).

use alloc::vec::Vec;

use std::ffi::OsString;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::{Child, Command};

use crate::ifac::Ifac;
use crate::node::InterfaceMode;

use super::interface::InterfaceId;
use super::mtu::HwMtu;
use super::pump::{Framing, PumpEnd, carry};
use super::runtime::{Endpoint, endpoint_closed, track};

/// The pipe's hardware MTU (`PipeInterface.py` 76).
pub const PIPE_HW_MTU: usize = 1064;
/// RNS's bitrate guess for a pipe (`PipeInterface.py` 56).
const PIPE_BITRATE_GUESS: u64 = 1_000_000;
/// Floor on `respawn_delay`, so a child that dies at once cannot spin the loop.
const MIN_RESPAWN_DELAY: Duration = Duration::from_millis(100);

/// A PipeInterface: RNS's `command` and `respawn_delay`.
#[derive(Clone, Debug)]
pub struct PipeConfig {
    /// The program and its arguments, already split; RNS splits `command` with `shlex`.
    pub argv: Vec<OsString>,
    /// Wait before respawning a child that exited (`PipeInterface.py` 59, 175-190); at
    /// least 100 ms.
    pub respawn_delay: Duration,
    /// Access code. A stock pipe without `ifac_size` uses 8 bytes (`DEFAULT_IFAC_SIZE`,
    /// `PipeInterface.py` 57), which is [`Ifac::for_serial`].
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

    fn spawn(&self) -> io::Result<Child> {
        let (program, args) = self
            .argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty pipe command"))?;
        Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    }
}

impl Endpoint {
    /// Spawn `config.argv` and carry an interface over its stdin and stdout, respawning it
    /// under the same id every `respawn_delay` after it exits, offline in between.
    ///
    /// Fails if the first spawn fails, as RNS's constructor raises. A child that closes its
    /// stdout or cannot be written to is killed and respawned; RNS instead raises on EOF
    /// (`PipeInterface.py` 146-151) and into Transport on a failed write.
    pub async fn attach_pipe(&self, config: PipeConfig) -> io::Result<InterfaceId> {
        if !self.shared.is_running() {
            return Err(endpoint_closed());
        }
        let mut child = Some(config.spawn()?);
        // Neither AUTOCONFIGURE_MTU nor FIXED_MTU: links stay at the protocol MTU.
        let attached = self.shared.add_interface(
            PIPE_HW_MTU,
            config.ifac.clone(),
            InterfaceMode::Full,
            true,
            HwMtu::Protocol,
        );
        if !attached.registered {
            return Err(endpoint_closed());
        }
        let (id, online, mut out) = (attached.id, attached.online, attached.outbound);
        self.set_interface_bitrate(id, Some(PIPE_BITRATE_GUESS));
        let shared = Arc::clone(&self.shared);
        let started = track(&self.shared, async move {
            loop {
                if let Some(mut running) = child.take() {
                    let (Some(stdout), Some(stdin)) = (running.stdout.take(), running.stdin.take())
                    else {
                        unreachable!("both pipes were requested");
                    };
                    let end = carry(&shared, id, &online, &mut out, Framing::Hdlc, stdout, stdin);
                    let end = end.await;
                    let _ = running.kill().await;
                    if end == PumpEnd::Closed {
                        return;
                    }
                }
                tokio::time::sleep(config.respawn_delay.max(MIN_RESPAWN_DELAY)).await;
                if shared.with_iface(id, |_| ()).is_none() {
                    return;
                }
                child = config.spawn().ok();
            }
        });
        if !started {
            self.shared.forget_interface(id);
            return Err(endpoint_closed());
        }
        Ok(id)
    }
}
