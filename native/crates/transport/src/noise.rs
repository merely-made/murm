// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Noise: an encrypted, mutually authenticated **session layer** that composes
//! over any byte stream.
//!
//! ## What this is, and what it is not
//!
//! This is not a carrier competing with the iroh lane. iroh is the byte plane:
//! it owns QUIC, NAT traversal, relays, and discovery, and nothing here moves
//! bytes off it. What the [Noise Protocol Framework](https://noiseprotocol.org/)
//! adds is a second, *application-layer* handshake that runs **inside** a
//! stream the carrier already opened.
//!
//! [`handshake`] is generic over `S: AsyncRead + AsyncWrite + Unpin`, which is
//! the whole design. Hand it an iroh stream and you get Noise over iroh; hand
//! it a TCP stream and you get the standalone case ([`NoiseListener`]); hand it
//! a Reticulum link and you get Noise over the mesh. The layer never learns
//! what is underneath it.
//!
//! ## What composing it over iroh buys
//!
//! **Zero trust through relays.** An iroh connection through a relay is
//! encrypted to the relay's satisfaction of QUIC, but the plane it traverses is
//! not ours. A Noise session inside it is opaque to everything between the two
//! endpoints, including infrastructure we run.
//!
//! **Layered identity.** This is the part worth being precise about, because
//! the two keys answer different questions:
//!
//! - The **carrier identity** (iroh's endpoint key, an Ed25519 public half)
//!   answers *where do packets go*. It is routing and traversal machinery, it
//!   is visible to relays, and it is long-lived because reachability depends on
//!   its stability.
//! - The **Noise identity** (whatever keypair is passed to [`handshake`])
//!   answers *who is speaking*. It can be a persona, one agent acting for a
//!   persona, or an identity that exists only for this session.
//!
//! Collapsing them is a choice, not a default: pass the same keypair and you
//! get one identity in two places. Pass [`Ed25519Keypair::derive_child`] or
//! [`Ed25519Keypair::generate`] and the peer learns who it is talking to
//! without learning which node, at which address, it reached them on.
//!
//! ## Two things this layer defines, because Noise does not
//!
//! **Framing.** Noise messages are discrete and capped at 65535 bytes; a
//! stream has no message boundaries. The specification leaves the
//! reconciliation to the application, so [`stream`] defines a 2-byte big-endian
//! length prefix, the conventional choice. Ours to make: both ends are this
//! code. It is not licence to guess another protocol's framing.
//!
//! **Identity.** Noise proves an X25519 static key; Mere's [`PeerID`] is an
//! Ed25519 key. So each side sends an identity proof as its first transport
//! message, signing this session's handshake hash. See [`keys`].
//!
//! ## The suite
//!
//! [`NOISE_PARAMS`]: `Noise_XX_25519_ChaChaPoly_BLAKE2b`. `XX` is the
//! mutual-unknown pattern: neither side needs the other's key in advance, both
//! transmit theirs encrypted during the handshake, and both end up holding the
//! other's proven static key. That is trust-on-first-use as a protocol
//! property rather than as a workaround.
//!
//! ## On BLAKE2b, since our stack is BLAKE3
//!
//! The hash is part of the suite name and is **not negotiated**, so both ends
//! must use BLAKE2b or they simply do not interoperate. BLAKE3 is not in the
//! Noise specification and `snow` does not offer it. This is not a gap to
//! close: the two hashes do different jobs. BLAKE3 is our *content addressing*
//! choice, which is ours precisely because it never appears on a wire. BLAKE2b
//! here is *wire description*, which is the peer's to dictate. Unifying them
//! would mean either breaking interoperability or letting a foreign protocol
//! choose our content-addressing hash.

mod keys;
mod stream;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;

use snow::Builder;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use identity::Ed25519Keypair;

use crate::{AcceptedSession, Alpn, IngressContext, PeerID, TransportError};

pub use keys::ProofError;
pub use stream::NoiseStream;

/// The Noise suite this layer speaks. Not negotiated: a peer built against a
/// different suite cannot complete a handshake, by design.
pub const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2b";

fn backend(context: &str, error: impl std::fmt::Display) -> TransportError {
    TransportError::Backend(format!("noise: {context}: {error}"))
}

/// Run the Noise `XX` handshake over `inner`, then exchange identity proofs.
///
/// This is the composable core: `inner` is any byte stream, so the same call
/// secures an iroh stream, a TCP socket, or a Reticulum link.
///
/// `identity` is the keypair this side proves to the peer, and it is
/// deliberately a parameter rather than the carrier's key. Passing a distinct
/// keypair -- [`Ed25519Keypair::derive_child`] for a durable application
/// identity, [`Ed25519Keypair::generate`] for one that lives only as long as
/// the session -- is what makes the identity a *layer* rather than a second
/// copy of the carrier's. Passing the carrier's own keypair is legitimate and
/// collapses the two.
///
/// The returned stream is encrypted and the returned [`PeerID`] is proven, not
/// claimed.
pub async fn handshake<S>(
    identity: &Ed25519Keypair,
    mut inner: S,
    initiator: bool,
) -> Result<(NoiseStream<S>, PeerID), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let params = NOISE_PARAMS.parse().map_err(|e| backend("params", e))?;
    let secret = keys::derive_static_secret(identity);

    let mut state = {
        let builder = Builder::new(params)
            .local_private_key(&secret)
            .map_err(|e| backend("static key", e))?;
        if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|e| backend("build", e))?
    };

    let mut scratch = vec![0u8; stream::MAX_MESSAGE];

    // XX is three messages: -> e, <- e ee s es, -> s se.
    while !state.is_handshake_finished() {
        if state.is_my_turn() {
            let written = state
                .write_message(&[], &mut scratch)
                .map_err(|e| backend("handshake write", e))?;
            stream::write_frame(&mut inner, &scratch[..written])
                .await
                .map_err(|e| backend("frame write", e))?;
        } else {
            let frame = stream::read_frame(&mut inner)
                .await
                .map_err(|e| backend("frame read", e))?;
            state
                .read_message(&frame, &mut scratch)
                .map_err(|e| backend("handshake read", e))?;
        }
    }

    // Both sides derive the same hash, and only a peer that completed this
    // handshake can sign it.
    let handshake_hash = state.get_handshake_hash().to_vec();
    let session = state
        .into_transport_mode()
        .map_err(|e| backend("transport mode", e))?;
    let mut stream = NoiseStream::new(inner, session);

    // The proof rides the encrypted session, so an observer never learns which
    // identities are talking. The initiator writes first to avoid a deadlock.
    let proof = keys::build_proof(identity, &handshake_hash);
    if initiator {
        write_len_prefixed(&mut stream, &proof)
            .await
            .map_err(|e| backend("proof write", e))?;
    }
    let theirs = read_len_prefixed(&mut stream, keys::PROOF_LEN)
        .await
        .map_err(|e| backend("proof read", e))?;
    if !initiator {
        write_len_prefixed(&mut stream, &proof)
            .await
            .map_err(|e| backend("proof write", e))?;
    }

    let peer =
        keys::verify_proof(&theirs, &handshake_hash).map_err(|e| backend("peer identity", e))?;

    Ok((stream, peer))
}

/// Secure a stream the carrier already opened, and announce a protocol.
///
/// The composition this module exists for: the carrier dialled and knows where
/// the bytes go; this adds the second handshake inside. Over iroh that means
/// the ALPN was already negotiated in QUIC's clear -- `alpn` here is a
/// *second*, encrypted declaration, which is what lets the two layers disagree
/// deliberately (one protocol visible to relays, another actually spoken).
pub async fn secure_initiator<S>(
    identity: &Ed25519Keypair,
    inner: S,
    alpn: &Alpn,
) -> Result<(NoiseStream<S>, PeerID), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut stream, peer) = handshake(identity, inner, true).await?;
    // Noise has no ALPN concept, so the protocol name is a framed message
    // inside the session. Sending it encrypted keeps which protocol is being
    // spoken confidential, unlike TLS ALPN.
    write_len_prefixed(&mut stream, alpn.as_bytes())
        .await
        .map_err(|e| backend("alpn write", e))?;
    Ok((stream, peer))
}

/// The responder half of [`secure_initiator`], returning the protocol asked for.
pub async fn secure_responder<S>(
    identity: &Ed25519Keypair,
    inner: S,
) -> Result<(NoiseStream<S>, PeerID, Alpn), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut stream, peer) = handshake(identity, inner, false).await?;
    let alpn = read_len_prefixed(&mut stream, 512)
        .await
        .map_err(|e| backend("alpn read", e))?;
    Ok((stream, peer, Alpn::from_bytes(alpn)))
}

/// Open a standalone Noise session over TCP: connect, handshake, announce.
pub async fn connect_to(
    identity: &Ed25519Keypair,
    addr: SocketAddr,
    alpn: &Alpn,
) -> Result<(NoiseStream<TcpStream>, PeerID), TransportError> {
    let tcp = TcpStream::connect(addr)
        .await
        .map_err(|e| backend("tcp connect", e))?;
    secure_initiator(identity, tcp, alpn).await
}

/// Accept one standalone TCP session, returning the ALPN the peer asked for.
pub async fn accept_from(
    identity: &Ed25519Keypair,
    listener: &TcpListener,
) -> Result<(NoiseStream<TcpStream>, PeerID, Alpn), TransportError> {
    let (tcp, _from) = listener
        .accept()
        .await
        .map_err(|e| backend("tcp accept", e))?;
    secure_responder(identity, tcp).await
}

/// Write a 2-byte-length-prefixed message *inside* the encrypted session.
///
/// The Noise framing below carries whole messages, but a byte-stream reader
/// still needs to know where one logical message ends, so short control
/// messages (the ALPN, the identity proof) carry their own length.
async fn write_len_prefixed<S>(stream: &mut S, message: &[u8]) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let length = u16::try_from(message.len())
        .map_err(|_| std::io::Error::other("control message exceeds 65535"))?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(message).await?;
    stream.flush().await
}

/// Read a length-prefixed control message, refusing anything over `max`.
async fn read_len_prefixed<S>(stream: &mut S, max: usize) -> std::io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0u8; 2];
    stream.read_exact(&mut length).await?;
    let length = usize::from(u16::from_be_bytes(length));
    if length > max {
        return Err(std::io::Error::other(format!(
            "control message of {length} bytes exceeds the {max}-byte limit"
        )));
    }
    let mut message = vec![0u8; length];
    stream.read_exact(&mut message).await?;
    Ok(message)
}

/// A TCP listener that speaks Noise: the standalone deployment, for links with
/// no carrier under them.
///
/// Deliberately **not** a [`Transport`](crate::Transport). A `Transport` dials
/// by [`PeerID`] because it owns discovery, and this owns none: it dials by
/// address, and an address has to come from somewhere else. Implementing the
/// trait would have meant a `connect` that always fails, which is a tell that
/// the trait is the wrong fit -- and it would have positioned Noise as a rival
/// byte plane to iroh, which it is not. Over iroh, use [`secure_initiator`] /
/// [`secure_responder`] on a stream the carrier opened.
pub struct NoiseListener {
    identity: Ed25519Keypair,
    listener: Arc<TcpListener>,
    local: PeerID,
    /// Sessions accepted while waiting for a different ALPN.
    parked: Mutex<Vec<(NoiseStream<TcpStream>, PeerID, Alpn)>>,
}

impl NoiseListener {
    /// Bind a standalone Noise listener on `addr`, proving `identity`.
    pub async fn bind(identity: Ed25519Keypair, addr: SocketAddr) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| backend("bind", e))?;
        Ok(Self {
            local: PeerID::from_public_key(identity.public_key()),
            identity,
            listener: Arc::new(listener),
            parked: Mutex::new(Vec::new()),
        })
    }

    /// The identity this listener proves to peers.
    pub fn local_peer_id(&self) -> PeerID {
        self.local
    }

    /// The address actually bound, which matters when binding port 0.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.listener
            .local_addr()
            .map_err(|e| backend("local addr", e))
    }

    /// Connect to a peer at a known address.
    pub async fn connect_addr(
        &self,
        addr: SocketAddr,
        alpn: &Alpn,
    ) -> Result<(NoiseStream<TcpStream>, PeerID), TransportError> {
        connect_to(&self.identity, addr, alpn).await
    }

    /// Accept the next session for `alpn`, parking any other ALPN's session
    /// for a later caller rather than dropping it.
    pub async fn accept_alpn(
        &self,
        alpn: &Alpn,
    ) -> Result<AcceptedSession<NoiseStream<TcpStream>>, TransportError> {
        let already_parked = {
            let mut parked = self.parked.lock().await;
            parked
                .iter()
                .position(|(_, _, a)| a == alpn)
                .map(|index| parked.remove(index))
                .map(|(stream, peer, _)| (stream, peer))
        };

        let (stream, peer) = match already_parked {
            Some(session) => session,
            None => loop {
                let (stream, peer, got) = accept_from(&self.identity, &self.listener).await?;
                if &got == alpn {
                    break (stream, peer);
                }
                self.parked.lock().await.push((stream, peer, got));
            },
        };

        Ok(AcceptedSession::new(
            stream,
            alpn.clone(),
            // Proven by the identity proof, never taken from application bytes.
            Some(peer),
            IngressContext::noise(),
        ))
    }
}
