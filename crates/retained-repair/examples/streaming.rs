// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Bounded interactive host delivery; no radio/network or application admission.

use ed25519_dalek::SigningKey;
use retained_repair::{
    Collector, Decision, Error, MAX_OBJECT_OCTETS, Producer, Profile, ReceiveOutcome,
};
use std::io::{BufRead, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = SigningKey::from_bytes(&[0x42; 32]); // Public fixture, never a credential.
    let profile = Profile::new([7; 32], key.verifying_key(), 4, 7, 1)?;
    let mut producer = Producer::new(profile.clone(), key)?;
    let mut fast = Collector::new(profile.clone())?;
    let mut late = Collector::new(profile)?;
    let mut now = 0u32;
    let mut join = 20u32;
    let mut input = std::io::stdin().lock();
    println!(
        "host logical clock; publish GEN EXPIRY HEX | tick COUNT | join TICK | inventory | quit"
    );
    for _ in 0..256 {
        let mut line = Vec::new();
        input
            .by_ref()
            .take((MAX_OBJECT_OCTETS * 2 + 256) as u64)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            break;
        }
        if line.len() >= MAX_OBJECT_OCTETS * 2 + 256 || !line.ends_with(b"\n") {
            return Err("command line too long or incomplete".into());
        }
        let line = std::str::from_utf8(&line)?;
        let args: Vec<_> = line.split_whitespace().collect();
        producer.advance(now)?;
        fast.advance(now)?;
        late.advance(now)?;
        match args.as_slice() {
            ["publish", generation, expiry, bytes] => {
                let bytes = unhex(bytes)?;
                println!(
                    "{:?}",
                    producer.publish(now, generation.parse()?, &bytes, expiry.parse()?)?
                );
            }
            ["join", tick] => {
                join = tick.parse()?;
                println!("late receiver joins at {join}");
            }
            ["tick", count] => {
                let count: u32 = count.parse()?;
                if count == 0 || count > 1024 {
                    return Err("tick count must be 1..=1024".into());
                }
                for _ in 0..count {
                    producer.advance(now)?;
                    fast.advance(now)?;
                    late.advance(now)?;
                    for channel in 0..4 {
                        if let Some(frame) = producer.prepare(now, channel, None)? {
                            producer.settle(now, frame.token, Decision::Accept)?;
                            emit("fast", now, fast.ingest(now, &frame.bytes)?);
                            let listen = if now.is_multiple_of(7) {
                                0
                            } else {
                                (now % 4) as u8
                            };
                            if now >= join && channel == listen {
                                match late.ingest(now, &frame.bytes) {
                                    Ok(outcome) => emit("late", now, outcome),
                                    Err(Error::UnknownGeneration) => {}
                                    Err(error) => return Err(error.into()),
                                }
                            }
                        }
                    }
                    now = now.checked_add(1).ok_or("clock overflow")?;
                }
                println!("now={now} {:?}", producer.stats());
            }
            ["inventory"] => println!("{:?}", producer.inventory()),
            ["quit"] => break,
            _ => return Err("unknown command".into()),
        }
    }
    Ok(())
}
fn emit(receiver: &str, now: u32, outcome: ReceiveOutcome) {
    if let ReceiveOutcome::Completed(object) = outcome {
        println!(
            "{receiver} complete now={now} generation={} octets={} digest={} exact={}",
            object.metadata.generation,
            object.bytes.len(),
            blake3::hash(&object.bytes),
            hex(&object.bytes)
        );
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if !text.len().is_multiple_of(2) || text.len() > MAX_OBJECT_OCTETS * 2 {
        return Err("hex length".into());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair)?;
            Ok(u8::from_str_radix(s, 16)?)
        })
        .collect()
}
