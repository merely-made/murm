//! The bridge: where a router's SAM port is, and (with `tokio`) the one-shot commands it
//! answers.

use std::net::{Ipv4Addr, SocketAddr};

use crate::{Error, Result};

/// A router's SAM bridge address. Any host may be given, not only loopback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bridge {
    addr: SocketAddr,
}

impl Default for Bridge {
    /// `127.0.0.1:7656`, the port Java I2P and i2pd use.
    fn default() -> Self {
        Self::new(SocketAddr::from((Ipv4Addr::LOCALHOST, Self::DEFAULT_PORT)))
    }
}

impl Bridge {
    /// The SAM port routers listen on by default.
    pub const DEFAULT_PORT: u16 = 7656;

    /// A bridge at `addr`.
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    /// The bridge named by `I2P_SAM_ADDRESS` (`host:port`), as i2plib and RNS read it, or
    /// the default when it is unset.
    pub fn from_env() -> Result<Self> {
        match std::env::var("I2P_SAM_ADDRESS") {
            Ok(text) => text
                .parse()
                .map(Self::new)
                .map_err(|_| Error::InvalidInput("I2P_SAM_ADDRESS is not host:port")),
            Err(_) => Ok(Self::default()),
        }
    }

    /// The bridge address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

#[cfg(feature = "tokio")]
mod commands {
    use super::Bridge;
    use crate::Result;
    use crate::control::Control;
    use crate::dest::{Destination, PrivateKey};
    use crate::proto::{self, Command};

    impl Bridge {
        pub(crate) async fn open(&self) -> Result<Control> {
            Control::open(self.addr).await
        }

        /// Generate a new destination with `signature_type` ([`crate::SIGNATURE_ED25519`] is
        /// what RNS uses).
        pub async fn generate(&self, signature_type: u16) -> Result<PrivateKey> {
            let mut control = self.open().await?;
            control
                .send(Command::DestGenerate { signature_type })
                .await?;
            proto::dest_reply(&control.read_line().await?)
        }

        /// Look up a name (`host.i2p`, `xxx.b32.i2p`) through the router.
        pub async fn lookup(&self, name: &str) -> Result<Destination> {
            // Validated before any connection is opened.
            Command::NamingLookup { name }.line()?;
            let mut control = self.open().await?;
            control.send(Command::NamingLookup { name }).await?;
            proto::naming_reply(&control.read_line().await?)
        }

        /// A destination given as base64, or else a name to look up.
        pub async fn resolve(&self, peer: &str) -> Result<Destination> {
            match Destination::from_base64(peer) {
                Ok(dest) => Ok(dest),
                Err(_) => self.lookup(peer).await,
            }
        }
    }
}
