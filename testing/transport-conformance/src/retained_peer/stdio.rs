// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant};

use super::actor::Actor;
use super::protocol::{MAX_LINE_BYTES, MAX_REQUESTS, MAX_TOTAL_BYTES, terminal_line};

pub const LIFETIME: Duration = Duration::from_secs(60);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const TERMINAL_RESERVE: usize = 1024;

pub(crate) enum Input {
    Line(Vec<u8>),
    Eof,
    Refused(&'static str, String),
}

/// Bound before JSON parsing. A queue holds at most one complete line; the
/// reader can additionally retain one next line and its fixed 4KiB buffer.
pub(crate) fn read_input<R: Read>(reader: R, sender: SyncSender<Input>) {
    let mut reader = BufReader::with_capacity(4096, reader.take((MAX_TOTAL_BYTES + 1) as u64));
    let mut line = Vec::new();
    let mut consumed = 0usize;
    let mut requests = 0usize;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = sender.send(Input::Refused("input_io", error.to_string()));
                return;
            }
        };
        if available.is_empty() {
            let event = if line.is_empty() {
                Input::Eof
            } else {
                Input::Refused("truncated_line", "EOF before newline terminator".into())
            };
            let _ = sender.send(event);
            return;
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if consumed + count > MAX_TOTAL_BYTES {
            let _ = sender.send(Input::Refused(
                "input_budget",
                "input lifetime byte budget exhausted".into(),
            ));
            return;
        }
        if line.len() + count > MAX_LINE_BYTES {
            let _ = sender.send(Input::Refused(
                "line_too_large",
                "encoded line exceeds 64KiB before JSON parsing".into(),
            ));
            return;
        }
        let complete = available[count - 1] == b'\n';
        line.extend_from_slice(&available[..count]);
        consumed += count;
        reader.consume(count);
        if complete {
            requests += 1;
            if requests > MAX_REQUESTS {
                let _ = sender.send(Input::Refused(
                    "request_budget",
                    "request count exhausted".into(),
                ));
                return;
            }
            // Preserve JSON whitespace; the newline was included in the byte budget.
            if sender.send(Input::Line(std::mem::take(&mut line))).is_err() {
                return;
            }
        }
    }
}

struct Output {
    bytes: Vec<u8>,
    acknowledged: SyncSender<Result<(), String>>,
}

fn start_writer() -> Result<SyncSender<Output>, String> {
    let (sender, receiver) = mpsc::sync_channel::<Output>(1);
    std::thread::Builder::new()
        .name("retained-peer-output".into())
        .spawn(move || {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            while let Ok(output) = receiver.recv() {
                let result = stdout
                    .write_all(&output.bytes)
                    .and_then(|()| stdout.flush())
                    .map_err(|error| error.to_string());
                let failed = result.is_err();
                let _ = output.acknowledged.send(result);
                if failed {
                    return;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(sender)
}

fn remaining(started: Instant, local: Duration) -> Result<Duration, String> {
    let remaining = LIFETIME
        .checked_sub(started.elapsed())
        .ok_or_else(|| "peer_lifetime: 60-second process lifetime expired".to_owned())?;
    Ok(remaining.min(local))
}

fn write_output(
    sender: &SyncSender<Output>,
    bytes: Vec<u8>,
    started: Instant,
) -> Result<(), String> {
    let (acknowledged, result) = mpsc::sync_channel(1);
    sender
        .try_send(Output {
            bytes,
            acknowledged,
        })
        .map_err(|_| "output worker unavailable".to_owned())?;
    result
        .recv_timeout(remaining(started, REQUEST_TIMEOUT)?)
        .map_err(|_| "output_timeout: output deadline expired or writer stopped".to_owned())?
}

fn close(
    sender: &SyncSender<Output>,
    code: &str,
    detail: &str,
    started: Instant,
    written: usize,
) -> Result<(), String> {
    let bytes = terminal_line(code, detail)?;
    if written + bytes.len() <= MAX_TOTAL_BYTES {
        // A terminal notice is best effort and never extends the whole lifetime.
        let _ = write_output(sender, bytes, started);
    }
    Err(format!("{code}: {detail}"))
}

/// Detached standard threads permit main to exit even when the parent leaves
/// stdin open or stops draining stdout. No thread is joined on an error path.
/// Main coordinates one request at a time; codec work and output each have a
/// bounded wait, independent of caller-supplied logical slots.
pub fn run_stdio() -> Result<(), String> {
    let started = Instant::now();
    let writer = start_writer()?;
    let (requests, incoming) = mpsc::sync_channel::<Vec<u8>>(1);
    let (results, outgoing) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);
    std::thread::Builder::new()
        .name("retained-peer-actor".into())
        .spawn(move || {
            let mut actor = match Actor::new() {
                Ok(actor) => actor,
                Err(error) => {
                    let _ = results.send(Err(error));
                    return;
                }
            };
            if results.send(actor.hello()).is_err() {
                return;
            }
            while let Ok(line) = incoming.recv() {
                if results.send(actor.handle_line(&line)).is_err() {
                    return;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    let hello = outgoing
        .recv_timeout(remaining(started, REQUEST_TIMEOUT)?)
        .map_err(|_| "startup_timeout: provenance/actor startup exceeded deadline".to_owned())??;
    let mut written = hello.len();
    write_output(&writer, hello, started)?;
    let (input, lines) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("retained-peer-input".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            read_input(stdin.lock(), input);
        })
        .map_err(|error| error.to_string())?;
    for _ in 0..=MAX_REQUESTS {
        let event = match lines.recv_timeout(remaining(started, IDLE_TIMEOUT)?) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return close(
                    &writer,
                    "idle_timeout",
                    "no next complete line within idle deadline",
                    started,
                    written,
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return close(
                    &writer,
                    "input_io",
                    "input reader stopped",
                    started,
                    written,
                );
            }
        };
        match event {
            Input::Eof => return Ok(()),
            Input::Refused(code, detail) => return close(&writer, code, &detail, started, written),
            Input::Line(line) => {
                requests
                    .try_send(line)
                    .map_err(|_| "actor request queue unavailable".to_owned())?;
                let response = match outgoing.recv_timeout(remaining(started, REQUEST_TIMEOUT)?) {
                    Ok(result) => result?,
                    Err(_) => {
                        return close(
                            &writer,
                            "request_timeout",
                            "actor request did not finish",
                            started,
                            written,
                        );
                    }
                };
                if written + response.len() > MAX_TOTAL_BYTES - TERMINAL_RESERVE {
                    return close(
                        &writer,
                        "output_budget",
                        "output lifetime byte budget exhausted",
                        started,
                        written,
                    );
                }
                written += response.len();
                write_output(&writer, response, started)?;
            }
        }
    }
    close(
        &writer,
        "request_budget",
        "request count exhausted",
        started,
        written,
    )
}
