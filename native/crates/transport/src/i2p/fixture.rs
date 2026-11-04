// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! A bounded scripted SAM bridge for tests. It is not an I2P router and
//! establishes no tunnel, anonymity, publication or public-network claim.

use super::I2pAddress;
use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use truchman::{Bridge, Destination};

type Arrival = (TcpStream, Destination);
struct Session {
    destination: Destination,
    waiting: VecDeque<oneshot::Sender<Arrival>>,
    stop: watch::Sender<bool>,
}
#[derive(Default)]
struct State {
    sessions: BTreeMap<String, Session>,
    sequence: u64,
    profiles: Vec<Vec<(String, String)>>,
    connects: usize,
}

/// A scripted provider and bounded task owner. Dropping it aborts all sockets.
pub struct SamFixture {
    address: SocketAddr,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}
impl SamFixture {
    /// Start on a fresh loopback port; no public network contact.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept(), if tasks.len() < 64 => {
                        let Ok((stream, _)) = accepted else { break };
                        tasks.spawn(connection(stream, shared.clone()));
                    },
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                }
            }
        });
        Self {
            address,
            state,
            task,
        }
    }
    /// Its sole loopback bridge.
    pub fn bridge(&self) -> Bridge {
        Bridge::new(self.address)
    }
    /// Captured session settings; private keys and full SAM responses are not retained.
    pub fn profiles(&self) -> Vec<Vec<(String, String)>> {
        self.state.lock().unwrap().profiles.clone()
    }
    /// Stream-connect attempts, useful for verifying refusal before network work.
    pub fn connects(&self) -> usize {
        self.state.lock().unwrap().connects
    }
    /// Terminate just one session's control channel, keeping the provider alive.
    pub fn terminate(&self, address: &I2pAddress) {
        for session in self.state.lock().unwrap().sessions.values() {
            if session.destination.b32() == address.as_str() {
                let _ = session.stop.send(true);
            }
        }
    }
    /// Stop and await the fixture owner, aborting its child tasks and sockets.
    pub async fn shutdown(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for SamFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|word| word.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
}
async fn line(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    match reader.read_line(&mut line).await {
        Ok(1..=16384) => Some(line),
        _ => None,
    }
}
fn destination(sequence: u64) -> (Destination, String) {
    let mut public = vec![0; 387];
    public[..8].copy_from_slice(&sequence.to_be_bytes());
    let dest = Destination::from_bytes(public.clone()).unwrap();
    public.extend([17; 32]);
    (dest, truchman::base64_encode(&public))
}
async fn connection(stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut reader = BufReader::new(stream);
    if !line(&mut reader)
        .await
        .is_some_and(|line| line.starts_with("HELLO VERSION"))
    {
        return;
    }
    if reader
        .get_mut()
        .write_all(b"HELLO REPLY RESULT=OK VERSION=3.3\n")
        .await
        .is_err()
    {
        return;
    }
    let Some(command) = line(&mut reader).await else {
        return;
    };
    if command.starts_with("SESSION CREATE") {
        let Some(id) = value(&command, "ID").map(str::to_string) else {
            return;
        };
        let (stop, mut stopped) = watch::channel(false);
        let private = {
            let mut state = state.lock().unwrap();
            if state.sessions.len() >= 16 {
                return;
            }
            state.sequence += 1;
            let (destination, private) = destination(state.sequence);
            state.profiles.push(
                command
                    .split_whitespace()
                    .filter_map(|part| part.split_once('='))
                    .filter(|(key, _)| {
                        key.starts_with("inbound.")
                            || key.starts_with("outbound.")
                            || key.starts_with("i2cp.")
                    })
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
            );
            state.sessions.insert(
                id.clone(),
                Session {
                    destination,
                    waiting: VecDeque::new(),
                    stop,
                },
            );
            private
        };
        let reply = format!("SESSION STATUS RESULT=OK DESTINATION={private}\n");
        if reader.get_mut().write_all(reply.as_bytes()).await.is_ok() {
            loop {
                let request = tokio::select! { request = line(&mut reader) => request, _ = stopped.changed() => None };
                let Some(request) = request else { break };
                if let Some(name) = value(&request, "NAME") {
                    let destination = state
                        .lock()
                        .unwrap()
                        .sessions
                        .values()
                        .find(|s| s.destination.b32() == name)
                        .map(|s| s.destination.to_base64());
                    let reply = match destination {
                        Some(dest) => format!("NAMING REPLY RESULT=OK NAME={name} VALUE={dest}\n"),
                        None => format!("NAMING REPLY RESULT=KEY_NOT_FOUND NAME={name}\n"),
                    };
                    if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        }
        state.lock().unwrap().sessions.remove(&id);
    } else if command.starts_with("STREAM ACCEPT") {
        let Some(id) = value(&command, "ID") else {
            return;
        };
        let (sender, arrival) = oneshot::channel();
        {
            let mut state = state.lock().unwrap();
            let Some(session) = state.sessions.get_mut(id) else {
                return;
            };
            if session.waiting.len() >= 16 {
                return;
            }
            session.waiting.push_back(sender);
        }
        if reader
            .get_mut()
            .write_all(b"STREAM STATUS RESULT=OK\n")
            .await
            .is_err()
        {
            return;
        }
        let arrived = tokio::select! {
            arrived = arrival => arrived,
            _ = reader.fill_buf() => return,
        };
        let Ok((mut outbound, peer)) = arrived else {
            return;
        };
        if reader
            .get_mut()
            .write_all(format!("{}\n", peer.to_base64()).as_bytes())
            .await
            .is_err()
        {
            return;
        }
        let _ = tokio::io::copy_bidirectional(reader.get_mut(), &mut outbound).await;
    } else if command.starts_with("STREAM CONNECT") {
        let Some(id) = value(&command, "ID") else {
            return;
        };
        let Some(destination) = value(&command, "DESTINATION") else {
            return;
        };
        let route = {
            let mut state = state.lock().unwrap();
            state.connects += 1;
            let Some(peer) = state.sessions.get(id).map(|s| s.destination.clone()) else {
                return;
            };
            state
                .sessions
                .values_mut()
                .find(|s| s.destination.to_base64() == destination)
                .and_then(|s| {
                    while let Some(waiting) = s.waiting.pop_front() {
                        if !waiting.is_closed() {
                            return Some(waiting);
                        }
                    }
                    None
                })
                .map(|waiting| (waiting, peer))
        };
        if let Some((waiting, peer)) = route {
            if reader
                .get_mut()
                .write_all(b"STREAM STATUS RESULT=OK\n")
                .await
                .is_err()
            {
                return;
            }
            let _ = waiting.send((reader.into_inner(), peer));
        } else {
            let _ = reader
                .get_mut()
                .write_all(b"STREAM STATUS RESULT=CANT_REACH_PEER\n")
                .await;
        }
    }
}
