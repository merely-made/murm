// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! What a transport bind leaves of its signing seed (vault lock ruling 51):
//! castellan's residue tracker, included by path, in one process.
//!
//! Each run binds and closes once, armed, as a spawned task (so its frames
//! are heap allocations the tracker sees, as in a resident), and lists the
//! blocks that held the seed when freed uncleared. Baselines come first:
//! iroh alone, and p2panda-net alone, built the way the transport builds
//! it, with every p2panda future and handle boxed where it is made. What the
//! transport adds beyond that baseline is its own: a block, freed or still
//! live, whose size is in every transport run and in no baseline run fails.
//! The transport also asks, between bind and close, each question that reads
//! iroh's endpoint. Each shape runs five times, so one-off scheduling blocks
//! (and the rare iroh task still holding its key when the run disarms) do
//! not count.
//!
//! The baseline also nests its bind and close as deep as the transport's.
//! p2panda-net leaves key bytes on the worker's stack while it binds, and a
//! later message to its endpoint actor (ractor boxes the whole 392-byte
//! enum, unused bytes included) can copy them to the heap; whether it does
//! depends on stack depth, so a shallower baseline would hide it.
//!
//! The default host policy turns mDNS on, so a third shape binds through it,
//! against p2panda-net with gossip and mDNS spawned in the transport's order
//! (ruling 60).
//!
//! No libtest harness: the allocator is process-wide.

#[path = "../../../../ports/castellan/tests/residue/mod.rs"]
mod residue;

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use muniment::{JsonCodec, MemoryBackend};
use p2panda_core::SigningKey;
use p2panda_net::address_book::AddressBookStoreHandle;
use p2panda_net::gossip::Gossip;
use p2panda_net::iroh_mdns::MdnsDiscoveryMode;
use p2panda_net::{AddressBook, Endpoint, MdnsDiscovery};
use residue::*;
use stickleback::MunimentAddressBook;
use transport::{P2pandaHostPolicy, P2pandaTransport, Transport};

const RUNS: usize = 5;

/// One armed run: the sizes of blocks freed uncleared, and of blocks still
/// live at disarm, that held the seed.
struct Run {
    freed: Vec<usize>,
    live: Vec<usize>,
}

fn measure<F>(rt: &tokio::runtime::Runtime, name: &str, work: impl Fn() -> F) -> Vec<Run>
where
    F: Future<Output = ()> + Send + 'static,
{
    (0..RUNS)
        .map(|_| {
            let future = work();
            arm();
            rt.block_on(async { tokio::spawn(future).await.unwrap() });
            // Let the runtime's workers finish what the close handed them.
            rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(300)).await });
            let (hits, overflow) = disarm();
            assert!(!overflow, "{name}: the allocation table overflowed");
            let sizes = |live: bool| {
                let mut sizes: Vec<usize> = hits
                    .iter()
                    .filter(|h| h.live == live)
                    .map(|h| h.size)
                    .collect();
                sizes.sort_unstable();
                sizes
            };
            let (freed, live) = (sizes(false), sizes(true));
            println!(
                "seed residue {name}: live {live:?}, freed uncleared {} {freed:?}",
                freed.len()
            );
            Run { freed, live }
        })
        .collect()
}

/// A connection hook that does nothing, standing in for the transport's.
#[derive(Clone, Debug)]
struct NoHook;

impl iroh::endpoint::EndpointHooks for NoHook {}

/// p2panda-net's endpoint spawn, built and boxed in a plain function.
#[allow(clippy::type_complexity)]
fn spawn_endpoint(
    address_book: AddressBook,
    seed: &[u8; 32],
) -> Pin<Box<impl Future<Output = Result<Endpoint, impl std::fmt::Debug>>>> {
    Box::pin(
        Endpoint::builder(address_book)
            .signing_key(SigningKey::from_bytes(seed))
            .hooks(NoHook)
            .spawn(),
    )
}

/// What a run turns on: gossip, and mDNS in the mode the default host
/// policy uses (ruling 60).
#[derive(Clone, Copy)]
struct Shape {
    gossip: bool,
    mdns: bool,
}

type Bound = (
    Box<Endpoint>,
    Option<Box<MdnsDiscovery>>,
    Option<Box<Gossip>>,
);

/// Two async layers, as the transport's `bind` and `bind_inner`.
async fn bind_outer(address_book: AddressBook, seed: &'static [u8; 32], shape: Shape) -> Bound {
    bind_like(address_book, seed, shape).await
}

async fn bind_like(address_book: AddressBook, seed: &'static [u8; 32], shape: Shape) -> Bound {
    let endpoint = Box::new(spawn_endpoint(address_book.clone(), seed).await.unwrap());
    // The transport spawns mDNS before gossip.
    let mdns = match shape.mdns {
        true => {
            let spawn = Box::pin(
                MdnsDiscovery::builder(address_book.clone(), (*endpoint).clone())
                    .mode(MdnsDiscoveryMode::Active)
                    .spawn(),
            );
            Some(Box::new(spawn.await.unwrap()))
        },
        false => None,
    };
    let overlay = match shape.gossip {
        true => {
            let spawn =
                Box::pin(Gossip::builder(address_book.clone(), (*endpoint).clone()).spawn());
            Some(Box::new(spawn.await.unwrap()))
        },
        false => None,
    };
    (endpoint, mdns, overlay)
}

/// The transport's `close`, line for line.
async fn close_like(endpoint: &Endpoint) -> Result<(), String> {
    let endpoint = endpoint
        .endpoint()
        .await
        .map_err(|e| format!("endpoint(): {e}"))?;
    endpoint.close().await;
    Ok(())
}

/// The baseline: p2panda-net built, nested and dropped as the transport does.
async fn p2panda_net(seed: &'static [u8; 32], shape: Shape) {
    let store = AddressBookStoreHandle::new(MunimentAddressBook::<MemoryBackend, JsonCodec>::new(
        MemoryBackend::new(),
    ));
    let address_book = AddressBook::builder().store(store).spawn().await.unwrap();
    let (endpoint, mdns, overlay) = bind_outer(address_book.clone(), seed, shape).await;
    close_like(&endpoint).await.unwrap();
    // The transport's field order.
    drop((endpoint, address_book, mdns, overlay));
}

/// The transport, awaited inline and held by value, as residents hold it.
/// Between bind and close it asks each question that reads iroh's endpoint
/// (ruling 54), so a message to p2panda's actor from any of them shows here.
/// With `mdns`, the builder goes through the default host policy, as every
/// overlay host's does (ruling 60).
async fn transport(seed: &'static [u8; 32], shape: Shape) {
    let builder = P2pandaTransport::builder_from_seed_ref(seed);
    let builder = match shape.gossip {
        true => builder.gossip(),
        false => builder,
    };
    let builder = match shape.mdns {
        true => P2pandaHostPolicy::default().configure(builder),
        false => builder,
    };
    let bound = builder.bind().await.unwrap();
    let me = bound.local_peer_id();
    bound.endpoint_addr().await.unwrap();
    bound.peers_for_topic([7; 32]).await.unwrap();
    bound.peer_ticket(me).await.unwrap();
    bound.peer_paths(me).await.unwrap();
    bound.close().await.unwrap();
    drop(bound);
}

/// iroh alone; with `mdns`, iroh's own mDNS lookup built as p2panda-net
/// builds it (from the public id, advertising), with no p2panda actor.
async fn iroh_alone(seed: &'static [u8; 32], mdns: bool) {
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(iroh::SecretKey::from_bytes(seed))
        .bind()
        .await
        .unwrap();
    let lookup = match mdns {
        true => Some(
            iroh_mdns_address_lookup::MdnsAddressLookup::builder()
                .advertise(true)
                .build(endpoint.id())
                .unwrap(),
        ),
        false => None,
    };
    endpoint.close().await;
    drop(lookup);
}

/// Blocks, `(live, size)`, in every transport run and in no baseline run.
fn transports_own(runs: &[Run], baseline: &[Run]) -> BTreeSet<(bool, usize)> {
    let blocks = |run: &Run| -> BTreeSet<(bool, usize)> {
        let freed = run.freed.iter().map(|&size| (false, size));
        freed
            .chain(run.live.iter().map(|&size| (true, size)))
            .collect()
    };
    let seen: BTreeSet<(bool, usize)> = baseline.iter().flat_map(blocks).collect();
    let mut always = blocks(&runs[0]);
    for run in &runs[1..] {
        always = always.intersection(&blocks(run)).copied().collect();
    }
    always.difference(&seen).copied().collect()
}

fn main() {
    let seed: [u8; 32] = canary_bytes(0);
    positive_control(&seed);
    clear_canaries();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    // Borrowed by every run; allocated before arming, so never scanned.
    let seed: &'static [u8; 32] = Box::leak(Box::new(seed));
    plant(0, "transport seed", seed, true);

    let iroh = measure(&rt, "iroh endpoint alone", || iroh_alone(seed, false));
    let iroh_mdns = measure(&rt, "iroh endpoint alone, mDNS", || iroh_alone(seed, true));
    let [plain, gossip, policy] = [
        Shape {
            gossip: false,
            mdns: false,
        },
        Shape {
            gossip: true,
            mdns: false,
        },
        Shape {
            gossip: true,
            mdns: true,
        },
    ];
    let net = measure(&rt, "p2panda-net alone", || p2panda_net(seed, plain));
    let net_gossip = measure(&rt, "p2panda-net alone, gossip", || {
        p2panda_net(seed, gossip)
    });
    let net_policy = measure(&rt, "p2panda-net alone, gossip, mDNS", || {
        p2panda_net(seed, policy)
    });
    let bare = measure(&rt, "transport", || transport(seed, plain));
    let with_gossip = measure(&rt, "transport, gossip", || transport(seed, gossip));
    let with_policy = measure(&rt, "transport, default host policy", || {
        transport(seed, policy)
    });
    clear_canaries();
    drop(rt);

    // iroh's own blocks are reported above, never failed on: they are its.
    // Which of the default policy's blocks iroh's mDNS lookup accounts for,
    // and which p2panda-net's mDNS actor adds (ruling 60), reported only.
    println!(
        "seed residue iroh mDNS lookup: blocks beyond iroh alone {:?}",
        transports_own(&iroh_mdns, &iroh)
    );
    println!(
        "seed residue p2panda-net mDNS: blocks beyond p2panda-net with gossip {:?}",
        transports_own(&net_policy, &net_gossip)
    );
    drop((iroh, iroh_mdns));
    let mut failures = Vec::new();
    for (name, runs, baseline) in [
        ("transport", &bare, &net),
        ("transport, gossip", &with_gossip, &net_gossip),
        ("transport, default host policy", &with_policy, &net_policy),
    ] {
        let own = transports_own(runs, baseline);
        println!("seed residue {name}: blocks (live, size) beyond p2panda-net {own:?}");
        if !own.is_empty() {
            failures.push(format!("{name}: the transport's own blocks {own:?}"));
        }
    }
    if !failures.is_empty() {
        println!("seed residue: FAILED {failures:?}");
        std::process::exit(1);
    }
    println!("seed residue: the transport adds nothing beyond p2panda-net");
}
