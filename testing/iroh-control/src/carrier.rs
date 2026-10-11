// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Probe-only framing: one command and one existing WN0 response per FIN-delimited stream.

use std::{
    fs,
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, ensure};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, endpoint::presets};
use postilion::control::verified::ControlExchange;
use retinue::command::MAX_COMMAND_LEN;
use seneschal::control::{MAX_RESPONSE_LEN, Response, decode_response, encode_response};
use tokio::{
    sync::{Semaphore, mpsc, oneshot},
    task::JoinSet,
    time::timeout,
};

use crate::board::Board;

pub const ALPN: &[u8] = b"retinue-probe/control/1";
pub const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SESSIONS: usize = 4;

pub async fn endpoint(server: bool) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind_addr("127.0.0.1:0".parse::<SocketAddr>()?)?;
    if server {
        // Known public test fixture; stable transport identity across fresh processes.
        builder = builder
            .secret_key(SecretKey::from_bytes(&[0x41; 32]))
            .alpns(vec![ALPN.to_vec()]);
    }
    Ok(builder.bind().await?)
}

#[derive(Clone)]
pub struct IrohCarrier {
    pub endpoint: Endpoint,
    pub destination: EndpointAddr,
}

impl ControlExchange for IrohCarrier {
    type Error = anyhow::Error;
    async fn exchange(&mut self, command: &[u8]) -> Result<Response> {
        ensure!(command.len() <= MAX_COMMAND_LEN, "oversized command");
        timeout(IO_TIMEOUT, async {
            let connection = self
                .endpoint
                .connect(self.destination.clone(), ALPN)
                .await?;
            let result = async {
                let (mut send, mut receive) = connection.open_bi().await?;
                send.write_all(command).await?;
                send.finish()?;
                let bytes = receive.read_to_end(MAX_RESPONSE_LEN).await?;
                decode_response(&bytes).map_err(|e| anyhow!("response: {e:?}"))
            }
            .await;
            connection.close(0u32.into(), b"exchange complete");
            result
        })
        .await
        .map_err(|_| anyhow!("carrier timeout"))?
    }
}

struct Exchange {
    wire: Vec<u8>,
    answer: oneshot::Sender<Result<Response>>,
}

pub async fn serve(directory: &Path) -> Result<()> {
    let mut board = Board::open(directory).await?;
    let started = Instant::now();
    let (tx, mut rx) = mpsc::channel::<Exchange>(MAX_SESSIONS);
    let board_task = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {
                _ = timer.tick() => board.expire(started.elapsed().as_millis() as u64).await?,
                exchange = rx.recv() => {
                    let Some(exchange) = exchange else { return Ok::<_, anyhow::Error>(()); };
                    // Complete and journal the admitted operation even if its client vanishes.
                    let result = board.command(&exchange.wire, started.elapsed().as_millis() as u64).await;
                    let _ = exchange.answer.send(result);
                }
            }
        }
    });
    let endpoint = endpoint(true).await?;
    let socket = endpoint
        .bound_sockets()
        .into_iter()
        .find(|socket| socket.is_ipv4())
        .ok_or_else(|| anyhow!("no IPv4 socket"))?;
    let address = EndpointAddr::new(endpoint.id()).with_ip_addr(socket);
    fs::write(directory.join("ready.tmp"), serde_json::to_vec(&address)?)?;
    fs::rename(directory.join("ready.tmp"), directory.join("ready.json"))?;
    let permits = Arc::new(Semaphore::new(MAX_SESSIONS));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break; };
                let Ok(permit) = permits.clone().try_acquire_owned() else { incoming.refuse(); continue; };
                let tx = tx.clone();
                let directory = directory.to_owned();
                tasks.spawn(async move {
                    let _permit = permit;
                    let result = timeout(IO_TIMEOUT, async {
                        let connection = incoming.await?;
                        let peer = connection.remote_id();
                        let (mut send, mut receive) = connection.accept_bi().await?;
                        let wire = match receive.read_to_end(MAX_COMMAND_LEN).await {
                            Ok(wire) => wire,
                            Err(error) => {
                                send.reset(2u32.into())?;
                                // A reset is queued, not delivered synchronously. Dropping
                                // the final connection handle here can replace it with a
                                // connection close before the peer observes the refusal.
                                connection.closed().await;
                                return Err(anyhow!("bounded input: {error}"));
                            }
                        };
                        let (answer, result) = oneshot::channel();
                        tx.send(Exchange { wire, answer }).await?;
                        match result.await? {
                            Ok(response) => {
                                // Fault injection occurs only after durable board execution.
                                if directory.join("drop-next-response").exists() {
                                    fs::remove_file(directory.join("drop-next-response"))?;
                                    send.reset(3u32.into())?;
                                } else {
                                    let mut bytes = [0; MAX_RESPONSE_LEN];
                                    let length = encode_response(&response, &mut bytes).map_err(|e| anyhow!("encode: {e:?}"))?;
                                    send.write_all(&bytes[..length]).await?;
                                    send.finish()?;
                                }
                            }
                            Err(error) => {
                                fs::write(directory.join("rejection.json"), serde_json::to_vec(&serde_json::json!({"peer": peer.to_string(), "reason": error.to_string()}))?)?;
                                send.reset(1u32.into())?;
                            }
                        }
                        // Keep the transport alive until the response/reset reaches the peer.
                        connection.closed().await;
                        Ok::<_, anyhow::Error>(())
                    }).await;
                    // Timeouts and peer disconnects are deliberate negative cases. Board
                    // errors are returned via the stream and separately retained above.
                    let _ = result;
                });
            }
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = result { return Err(error.into()); }
            }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {
                ensure!(!board_task.is_finished(), "board actor stopped");
            }
        }
    }
    board_task.abort();
    endpoint.close().await;
    Ok(())
}
