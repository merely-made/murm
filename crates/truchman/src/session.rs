//! STREAM sessions over tokio. A session lives as long as the connection that created it, so
//! it keeps that connection in a task that answers the bridge's `PING`s, carries the
//! session's name lookups, and notices when it closes.

use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot, watch};
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
    lookups: mpsc::UnboundedSender<Lookup>,
    control: JoinHandle<()>,
}

/// A name to look up on the session's control connection, and where its answer goes.
type Lookup = (String, oneshot::Sender<Result<Destination>>);

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
        let (lookups, requests) = mpsc::unbounded_channel();
        let control = tokio::spawn(serve_control(control, requests, tx));
        Ok(Self {
            bridge,
            id,
            key,
            alive,
            lookups,
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

    /// Look up a name (`xxx.b32.i2p`, a host name) on the session's own control connection,
    /// as SAMv3 allows. Routers that answer `NAMING LOOKUP` only inside a session, such as
    /// emissary, resolve `.b32.i2p` names this way through a lease-set query.
    pub async fn lookup(&self, name: &str) -> Result<Destination> {
        Command::NamingLookup { name }.line()?;
        let (answer, answered) = oneshot::channel();
        self.lookups
            .send((name.to_string(), answer))
            .map_err(|_| closed())?;
        answered.await.map_err(|_| closed())?
    }

    /// A destination given as base64, or else a name looked up through this session.
    pub async fn resolve(&self, peer: &str) -> Result<Destination> {
        match Destination::from_base64(peer) {
            Ok(dest) => Ok(dest),
            Err(_) => self.lookup(peer).await,
        }
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

/// The session's control connection: answer `PING`, write lookups and hand each `NAMING
/// REPLY` to the request for its `NAME`, until the bridge closes it.
async fn serve_control(
    control: crate::control::Control,
    mut requests: mpsc::UnboundedReceiver<Lookup>,
    alive: watch::Sender<bool>,
) {
    let (mut reader, mut writer) = control.into_split();
    let mut pending: Vec<(String, oneshot::Sender<Result<Destination>>)> = Vec::new();
    loop {
        tokio::select! {
            line = reader.read_line() => {
                let Ok(line) = line else { break };
                if let Some(rest) = line.strip_prefix("PING") {
                    let Ok(pong) = Command::Pong { rest }.line() else { continue };
                    if writer.write_all(pong.as_bytes()).await.is_err() {
                        break;
                    }
                } else if line.starts_with("NAMING REPLY") {
                    let name = crate::Reply::parse(&line)
                        .ok()
                        .and_then(|r| r.get("NAME").map(str::to_string));
                    if let Some(at) = pending.iter().position(|(n, _)| Some(n) == name.as_ref()) {
                        let (_, answer) = pending.swap_remove(at);
                        let _ = answer.send(proto::naming_reply(&line));
                    }
                }
            }
            Some((name, answer)) = requests.recv() => {
                let line = match (Command::NamingLookup { name: &name }).line() {
                    Ok(line) => line,
                    Err(e) => {
                        let _ = answer.send(Err(e));
                        continue;
                    }
                };
                if writer.write_all(line.as_bytes()).await.is_err() {
                    let _ = answer.send(Err(closed()));
                    break;
                }
                pending.push((name, answer));
            }
        }
    }
    for (_, answer) in pending {
        let _ = answer.send(Err(closed()));
    }
    let _ = alive.send(false);
}

fn closed() -> Error {
    Error::Io(std::io::ErrorKind::NotConnected.into())
}
