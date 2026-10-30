// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Fixture-owned ordered carrier adapters. No facts are read from peer claims.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result};
use notochord::{CarrierKind, ProofBinding, SessionFacts};
use personae::Ed25519Keypair;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use transport::{
    Alpn, OutboundSession, P2pandaStream, PeerID, ReticulumInterface, ReticulumTransport,
    Transport, p2panda_transport::P2pandaTransport,
};

pub trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}
pub type Stream = Box<dyn Io>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contact {
    pub carrier: String,
    pub route: String,
    pub publisher: [u8; 32],
}

pub enum Owner {
    Tcp(Option<TcpListener>),
    Iroh(P2pandaTransport),
    Reticulum(ReticulumTransport),
}

pub struct Incoming {
    pub stream: Stream,
    pub facts: SessionFacts,
}

pub struct Outgoing {
    pub stream: Stream,
    pub binding: ProofBinding,
    pub flip_next_ciphertext: Arc<AtomicBool>,
}

impl Owner {
    pub async fn listen(carrier: &str, protocol: &Alpn) -> Result<(Self, Contact)> {
        let key = super::server_key();
        let publisher = key.public_key().to_bytes();
        match carrier {
            "tcp" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let route = listener.local_addr()?.to_string();
                Ok((
                    Self::Tcp(Some(listener)),
                    Contact {
                        carrier: carrier.into(),
                        route,
                        publisher,
                    },
                ))
            }
            "iroh" | "iroh-native" => {
                let transport = P2pandaTransport::builder_from_seed(key.to_seed())
                    .alpns(vec![protocol.clone()])
                    .bind()
                    .await?;
                let route = transport.ticket().await?;
                Ok((
                    Self::Iroh(transport),
                    Contact {
                        carrier: carrier.into(),
                        route,
                        publisher,
                    },
                ))
            }
            "reticulum" => {
                let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
                let address = socket.local_addr()?;
                drop(socket);
                let carrier_key = Ed25519Keypair::from_seed([41; 32]);
                let transport = ReticulumTransport::builder(&carrier_key)
                    .alpns(vec![protocol.clone()])
                    .interfaces(vec![ReticulumInterface::TcpServer { bind: address }])
                    .announce_interval(Duration::from_millis(100))
                    .reliable_links(true)
                    .bind()
                    .await?;
                // The reached routing identity differs from the Noise publisher.
                Ok((
                    Self::Reticulum(transport),
                    Contact {
                        carrier: carrier.into(),
                        route: address.to_string(),
                        publisher,
                    },
                ))
            }
            _ => anyhow::bail!("unknown carrier {carrier}"),
        }
    }

    pub async fn connector(contact: &Contact, protocol: &Alpn, wrong_client: bool) -> Result<Self> {
        match contact.carrier.as_str() {
            "tcp" => Ok(Self::Tcp(None)),
            "iroh" | "iroh-native" => {
                let seed = if contact.carrier == "iroh-native" && wrong_client {
                    [3; 32]
                } else {
                    super::client_key().to_seed()
                };
                let transport = P2pandaTransport::builder_from_seed(seed)
                    .alpns(vec![protocol.clone()])
                    .bind()
                    .await?;
                transport.add_peer_ticket(&contact.route).await?;
                Ok(Self::Iroh(transport))
            }
            "reticulum" => {
                let carrier_key = Ed25519Keypair::from_seed([42; 32]);
                let transport = ReticulumTransport::builder(&carrier_key)
                    .alpns(vec![protocol.clone()])
                    .interfaces(vec![ReticulumInterface::TcpClient {
                        addr: contact.route.parse()?,
                    }])
                    .announce_interval(Duration::from_millis(100))
                    .reliable_links(true)
                    .bind()
                    .await?;
                Ok(Self::Reticulum(transport))
            }
            _ => anyhow::bail!("unknown carrier {}", contact.carrier),
        }
    }

    pub async fn accept(&self, protocol: &Alpn) -> Result<Incoming> {
        match self {
            Self::Tcp(Some(listener)) => {
                let (stream, _) = listener.accept().await?;
                // This fixture adapter owns the raw TCP observation. TCP proves
                // no initiator key or shared cryptographic link identity.
                Ok(Incoming {
                    stream: Box::new(stream),
                    facts: SessionFacts::new(protocol.as_bytes(), CarrierKind::Other),
                })
            }
            Self::Iroh(transport) => {
                let (stream, facts) = transport.accept(protocol.clone()).await?.into_session();
                Ok(Incoming {
                    stream: Box::new(stream),
                    facts,
                })
            }
            Self::Reticulum(transport) => {
                let (stream, facts) = transport.accept(protocol.clone()).await?.into_session();
                Ok(Incoming {
                    stream: Box::new(stream),
                    facts,
                })
            }
            Self::Tcp(None) => anyhow::bail!("client has no listener"),
        }
    }

    /// Retain the concrete backend stream until N2 obtains its private native
    /// channel evidence. Boxing it before this step would erase that authority.
    pub async fn accept_native(&self, protocol: &Alpn) -> Result<(P2pandaStream, SessionFacts)> {
        let Self::Iroh(transport) = self else {
            anyhow::bail!("native evidence is qualified only for iroh")
        };
        Ok(transport.accept(protocol.clone()).await?.into_session())
    }

    pub async fn connect_native(
        &self,
        contact: &Contact,
        protocol: &Alpn,
    ) -> Result<OutboundSession<P2pandaStream>> {
        let Self::Iroh(transport) = self else {
            anyhow::bail!("native evidence is qualified only for iroh")
        };
        let peer = PeerID::from_bytes(&contact.publisher).context("publisher identity")?;
        Ok(transport.connect_session(peer, protocol.clone()).await?)
    }

    pub async fn connect(&self, contact: &Contact, protocol: &Alpn) -> Result<Outgoing> {
        let (stream, binding): (Stream, ProofBinding) = match self {
            Self::Tcp(_) => (
                Box::new(TcpStream::connect(&contact.route).await?),
                ProofBinding::initiator(protocol.as_bytes(), None, None),
            ),
            Self::Iroh(transport) => {
                let peer = PeerID::from_bytes(&contact.publisher).context("publisher identity")?;
                let outgoing = transport.connect_session(peer, protocol.clone()).await?;
                let binding = outgoing.proof_binding();
                (Box::new(outgoing.stream), binding)
            }
            Self::Reticulum(transport) => {
                let routing =
                    PeerID::from_public_key(Ed25519Keypair::from_seed([41; 32]).public_key());
                let outgoing = transport.connect_session(routing, protocol.clone()).await?;
                let binding = outgoing.proof_binding();
                (Box::new(outgoing.stream), binding)
            }
        };
        let flip_next_ciphertext = Arc::new(AtomicBool::new(false));
        Ok(Outgoing {
            stream: Box::new(FlipCiphertext {
                inner: stream,
                armed: Arc::clone(&flip_next_ciphertext),
            }),
            binding,
            flip_next_ciphertext,
        })
    }

    pub async fn close(&self) -> Result<()> {
        if let Self::Iroh(transport) = self {
            transport.close().await?;
        }
        Ok(())
    }
}

/// A fixture mutation below Noise, armed only after secure setup. Length
/// prefixes remain intact; one ciphertext byte is changed on a successful write.
struct FlipCiphertext {
    inner: Stream,
    armed: Arc<AtomicBool>,
}

impl AsyncRead for FlipCiphertext {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for FlipCiphertext {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if data.len() > 2 && self.armed.load(Ordering::SeqCst) {
            let mut changed = data.to_vec();
            changed[0] ^= 1;
            let result = Pin::new(&mut self.inner).poll_write(cx, &changed);
            if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
                self.armed.store(false, Ordering::SeqCst);
            }
            result
        } else {
            Pin::new(&mut self.inner).poll_write(cx, data)
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
