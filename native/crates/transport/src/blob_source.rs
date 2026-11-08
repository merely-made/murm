// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! What a scoped blob provider serves from (cycle breaks P2 step 3).
//!
//! [`P2pandaTransportBuilder::scoped_source`](crate::p2panda_transport::P2pandaTransportBuilder::scoped_source)
//! answers iroh-blobs requests for one [`BlobScope`] from a [`BlobSource`]
//! instead of from a [`BlobStore`] in its own process. A process that does not
//! hold the store (a supervised application whose custody is a resident's)
//! implements the trait over whatever reaches the store, and its peers still
//! fetch verified streams.
//!
//! The trait is three questions: may this peer have this hash from this
//! scope, which hashes does this hash sequence name, and what is the verified
//! stream for these ranges of this blob. The third answer is exactly the bytes
//! iroh-blobs writes after a request for one blob (the BLAKE3 size header,
//! hash pairs and leaves), produced by whoever holds the data and checked by
//! the fetching peer against the hash it asked for, so a source in the middle
//! is trusted for availability and for nothing else.
//!
//! The provider keeps the rules the store-backed one had: read-only, every
//! named hash (and every child a hash-sequence request reaches) authorised
//! before any byte is written, pushes and unknown requests refused.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use iroh_blobs::api::Store;
pub use iroh_blobs::protocol::ChunkRanges;
use iroh_blobs::protocol::Request;
use n0_future::StreamExt;

use crate::blobs::{BlobHash, BlobReadAuthorizer, BlobScope, BlobStore};

/// The verified stream of one blob's requested ranges, in order, in pieces.
#[async_trait]
pub trait BaoReader: Send {
    /// The next piece, or `None` at the end.
    async fn next(&mut self) -> io::Result<Option<Bytes>>;
}

/// What a scoped provider reads from.
#[async_trait]
pub trait BlobSource: Send + Sync + 'static {
    /// Whether `peer` may fetch `hash` from `scope` now. Asked before every
    /// request and again for each child a hash-sequence request reaches.
    async fn allows(&self, scope: BlobScope, peer: &[u8; 32], hash: BlobHash) -> bool;

    /// The hashes a complete hash sequence names, in order.
    async fn children(&self, hash: BlobHash) -> io::Result<Vec<BlobHash>>;

    /// The iroh-blobs response bytes for `ranges` of the blob `hash`: the
    /// 8-byte little-endian size, then pre-order hash pairs and leaf data.
    /// `scope` and `peer` say whom it is for, so a source that holds custody
    /// elsewhere can re-check the same question at each piece.
    async fn export_bao(
        &self,
        scope: BlobScope,
        peer: &[u8; 32],
        hash: BlobHash,
        ranges: ChunkRanges,
    ) -> io::Result<Box<dyn BaoReader>>;
}

/// A [`BlobSource`] over a [`BlobStore`] in this process and the in-process
/// authorizer: what `scoped_blobs` serves from.
#[derive(Clone)]
pub struct StoreBlobSource {
    store: Store,
    authorizer: BlobReadAuthorizer,
}

impl StoreBlobSource {
    pub fn new(blobs: &BlobStore, authorizer: BlobReadAuthorizer) -> Self {
        Self {
            store: blobs.store().clone(),
            authorizer,
        }
    }
}

struct StoreBao(std::pin::Pin<Box<dyn n0_future::Stream<Item = io::Result<Bytes>> + Send>>);

#[async_trait]
impl BaoReader for StoreBao {
    async fn next(&mut self) -> io::Result<Option<Bytes>> {
        self.0.next().await.transpose()
    }
}

#[async_trait]
impl BlobSource for StoreBlobSource {
    async fn allows(&self, scope: BlobScope, peer: &[u8; 32], hash: BlobHash) -> bool {
        self.authorizer.allows(scope, peer, hash)
    }

    async fn children(&self, hash: BlobHash) -> io::Result<Vec<BlobHash>> {
        let bytes = self
            .store
            .blobs()
            .get_bytes(hash)
            .await
            .map_err(io::Error::other)?;
        let seq = iroh_blobs::hashseq::HashSeq::try_from(bytes)
            .map_err(|_| io::Error::other("not a hash sequence"))?;
        Ok(seq.iter().map(BlobHash::from).collect())
    }

    async fn export_bao(
        &self,
        _scope: BlobScope,
        _peer: &[u8; 32],
        hash: BlobHash,
        ranges: ChunkRanges,
    ) -> io::Result<Box<dyn BaoReader>> {
        let stream = self
            .store
            .blobs()
            .export_bao(hash, ranges)
            .into_byte_stream()
            .map(|item| item.map_err(io::Error::other));
        Ok(Box::new(StoreBao(Box::pin(stream))))
    }
}

/// One blob a request asks for, in wire order.
pub(crate) struct Wanted {
    pub hash: BlobHash,
    pub ranges: ChunkRanges,
}

/// Authorise `request` and say what to write, in order. `None` refuses the
/// whole request: nothing is written for a request any part of which is not
/// allowed, and only reads (`Get`, `GetMany`) are served.
pub(crate) async fn plan_request(
    source: &dyn BlobSource,
    scope: BlobScope,
    peer: &[u8; 32],
    request: &Request,
) -> Option<Vec<Wanted>> {
    match request {
        Request::Get(request) => {
            let root = BlobHash::from(request.hash);
            if !source.allows(scope, peer, root).await {
                return None;
            }
            if request.ranges.is_blob() {
                let (_, ranges) = request.ranges.iter_non_empty_infinite().next()?;
                return Some(vec![Wanted {
                    hash: root,
                    ranges: ranges.clone(),
                }]);
            }
            let children = source.children(root).await.ok()?;
            let mut wanted = Vec::new();
            for (offset, ranges) in request
                .ranges
                .iter_infinite()
                .take(children.len() + 1)
                .enumerate()
            {
                if ranges.is_empty() {
                    continue;
                }
                let hash = if offset == 0 {
                    root
                } else {
                    let child = *children.get(offset - 1)?;
                    if !source.allows(scope, peer, child).await {
                        return None;
                    }
                    child
                };
                wanted.push(Wanted {
                    hash,
                    ranges: ranges.clone(),
                });
            }
            Some(wanted)
        },
        Request::GetMany(request) => {
            let mut wanted = Vec::new();
            for (hash, ranges) in request.hashes.iter().zip(request.ranges.iter_infinite()) {
                if ranges.is_empty() {
                    continue;
                }
                let hash = BlobHash::from(*hash);
                if !source.allows(scope, peer, hash).await {
                    return None;
                }
                wanted.push(Wanted {
                    hash,
                    ranges: ranges.clone(),
                });
            }
            Some(wanted)
        },
        // Scoped serving is read-only; observe, push and unknown slots are refused.
        _ => None,
    }
}

/// Chunk ranges as `(start, end)` chunk-number intervals, `end` absent for
/// a range open to the end of the blob: the wire form a door carries.
pub fn chunk_ranges_to_intervals(ranges: &ChunkRanges) -> Vec<(u64, Option<u64>)> {
    ranges
        .boundaries()
        .chunks(2)
        .map(|pair| (pair[0].0, pair.get(1).map(|end| end.0)))
        .collect()
}

/// The inverse of [`chunk_ranges_to_intervals`]. Intervals must be
/// non-empty, ascending and disjoint, and only the last may be open.
pub fn intervals_to_chunk_ranges(intervals: &[(u64, Option<u64>)]) -> Result<ChunkRanges, String> {
    use bao_tree::ChunkNum;
    let mut ranges = ChunkRanges::empty();
    let mut floor = 0u64;
    for (index, (start, end)) in intervals.iter().enumerate() {
        if *start < floor {
            return Err("intervals must ascend and not overlap".into());
        }
        let piece = match end {
            Some(end) if end > start => ChunkRanges::from(ChunkNum(*start)..ChunkNum(*end)),
            Some(_) => return Err("an interval must not be empty".into()),
            None if index + 1 == intervals.len() => ChunkRanges::from(ChunkNum(*start)..),
            None => return Err("only the last interval may be open".into()),
        };
        floor = end.unwrap_or(u64::MAX);
        ranges |= piece;
    }
    Ok(ranges)
}

/// A shared source, as the builder holds it.
pub type SharedBlobSource = Arc<dyn BlobSource>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_round_trip_and_refuse_the_malformed() {
        let ranges = intervals_to_chunk_ranges(&[(0, Some(2)), (5, Some(9)), (12, None)]).unwrap();
        assert_eq!(
            chunk_ranges_to_intervals(&ranges),
            vec![(0, Some(2)), (5, Some(9)), (12, None)]
        );
        assert_eq!(
            chunk_ranges_to_intervals(&ChunkRanges::all()),
            vec![(0, None)]
        );
        assert!(chunk_ranges_to_intervals(&ChunkRanges::empty()).is_empty());
        assert!(intervals_to_chunk_ranges(&[(3, Some(3))]).is_err());
        assert!(intervals_to_chunk_ranges(&[(4, Some(6)), (5, Some(9))]).is_err());
        assert!(intervals_to_chunk_ranges(&[(0, None), (5, Some(9))]).is_err());
    }
}
