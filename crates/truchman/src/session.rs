//! STREAM sessions over tokio. A session lives as long as the connection that created it, so
//! it keeps that connection in a task that answers the bridge's `PING`s and notices when it
//! closes.

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::bridge::Bridge;
use crate::dest::{Destination, PrivateKey};
use crate::proto::{self, Command, Key, SessionOptions};
use crate::stream::Stream;
use crate::{Error, Result};

/// An open STREAM session. Dropping it closes the session.
#[derive(Debug)]
pub struct StreamSession {
    bridge: Bridge,
    id: String,
    key: PrivateKey,
    alive: watch::Receiver<bool>,
    control: JoinHandle<()>,
}

impl StreamSession {
    /// `SESSION CREATE STYLE=STREAM` on `bridge`. Building the session's tunnels can take the
    /// router a while; the call returns when the bridge answers.
    pub async fn create(bridge: Bridge, key: Key, options: SessionOptions) -> Result<Self> {
        let id = options.nickname.clone().unwrap_or_else(proto::nickname);
        let create = Command::SessionCreate {
            id: &id,
            key: &key,
            options: &options,
        };
        // Validated before any connection is opened.
        create.line()?;
        let mut control = bridge.open().await?;
        control.send(create).await?;
        let transient = proto::session_reply(&control.read_line().await?)?;
        let key = match key {
            Key::Persistent(private) => private,
            Key::Transient => PrivateKey::from_base64(
                &transient
                    .ok_or_else(|| Error::Protocol("SESSION STATUS without DESTINATION".into()))?,
            )?,
        };
        let (tx, alive) = watch::channel(true);
        let control = tokio::spawn(async move {
            while let Ok(line) = control.read_line().await {
                if let Some(rest) = line.strip_prefix("PING")
                    && control.send(Command::Pong { rest }).await.is_err()
                {
                    break;
                }
            }
            let _ = tx.send(false);
        });
        Ok(Self {
            bridge,
            id,
            key,
            alive,
            control,
        })
    }

    /// The session ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The session's destination.
    pub fn destination(&self) -> &Destination {
        self.key.destination()
    }

    /// The session's private key, to store a transient destination for reuse.
    pub fn private_key(&self) -> &PrivateKey {
        &self.key
    }

    /// The bridge the session lives on.
    pub fn bridge(&self) -> Bridge {
        self.bridge
    }

    /// Whether the bridge still holds the session's connection open.
    pub fn is_alive(&self) -> bool {
        *self.alive.borrow()
    }

    /// Wait until the bridge closes the session.
    pub async fn closed(&self) {
        let _ = self.alive.clone().wait_for(|alive| !alive).await;
    }

    /// Open a stream to `to` (`STREAM CONNECT`).
    pub async fn connect(&self, to: &Destination) -> Result<Stream> {
        let mut control = self.bridge.open().await?;
        let connect = Command::StreamConnect { id: &self.id, to };
        control.call(connect, "STREAM", "STATUS").await?;
        Ok(control.into_stream())
    }

    /// Wait for one incoming stream (`STREAM ACCEPT`), returning it with the peer's
    /// destination.
    pub async fn accept(&self) -> Result<(Stream, Destination)> {
        let mut control = self.bridge.open().await?;
        let accept = Command::StreamAccept { id: &self.id };
        control.call(accept, "STREAM", "STATUS").await?;
        let peer = proto::accepted_peer(&control.read_line().await?)?;
        Ok((control.into_stream(), peer))
    }
}

impl Drop for StreamSession {
    fn drop(&mut self) {
        self.control.abort();
    }
}
