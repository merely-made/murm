//! A SAMv3 client for I2P routers: a sans-io core with a tokio shell.
//!
//! SAM is the plain-text control protocol an I2P router (Java I2P, i2pd, emissary) offers
//! applications on a local TCP port. The core, always built, turns typed [`Command`]s into
//! validated lines and bridge bytes into [`Reply`]s through [`LineBuf`], which keeps whatever
//! followed a reply for the data phase. It also holds [`Destination`] and [`PrivateKey`]
//! parsing and I2P's base64/base32. The `tokio` feature (default) adds the I/O: [`Bridge`]'s
//! one-shot commands, [`StreamSession`], and the connected [`Stream`].
//!
//! Every value written onto a line is validated first, so a configured name, key or option
//! can never smuggle a second command onto it.
//!
//! Datagram sessions are not implemented yet.

mod bridge;
mod dest;
mod encoding;
mod error;
pub mod proto;
mod reply;

#[cfg(feature = "tokio")]
mod control;
#[cfg(feature = "tokio")]
mod session;
#[cfg(feature = "tokio")]
mod stream;

pub use bridge::Bridge;
pub use dest::{Destination, PrivateKey, SIGNATURE_ED25519};
pub use encoding::{base32_encode, base64_decode, base64_encode};
pub use error::Error;
pub use proto::{Command, Key, LineBuf, SessionOptions};
pub use reply::Reply;
#[cfg(feature = "tokio")]
pub use session::StreamSession;
#[cfg(feature = "tokio")]
pub use stream::{Prefixed, ReadHalf, Stream, WriteHalf};

/// The crate's result type.
pub type Result<T> = core::result::Result<T, Error>;
