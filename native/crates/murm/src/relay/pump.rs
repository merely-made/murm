// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::budget::Reservation;
use super::{RelayError, RelayGrant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, interval, sleep_until};

async fn direction<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut read: R,
    mut write: W,
    reservation: &Reservation,
) -> Result<(), RelayError> {
    let mut buffer = vec![0; reservation.buffer_size()];
    loop {
        let count = read.read(&mut buffer).await?;
        if count == 0 {
            write.shutdown().await?;
            return Ok(());
        }
        reservation.charge(count).await?;
        write.write_all(&buffer[..count]).await?;
        write.flush().await?;
    }
}

pub(crate) async fn forward<A, B, F>(
    a: A,
    b: B,
    reservation: &Reservation,
    original: RelayGrant,
    mut authority: F,
) -> Result<(), RelayError>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
    F: FnMut() -> Option<RelayGrant>,
{
    let (ar, aw) = tokio::io::split(a);
    let (br, bw) = tokio::io::split(b);
    let copy = async {
        tokio::try_join!(
            direction(ar, bw, reservation),
            direction(br, aw, reservation)
        )
        .map(|_| ())
    };
    let deadline = original
        .expires
        .min(reservation.started + reservation.limits.lifetime);
    let check = async {
        let mut tick = interval(reservation.limits.authority_interval);
        loop {
            tick.tick().await;
            let live = authority().ok_or(RelayError::Unauthorized)?;
            if live.revision != original.revision
                || live.max_sessions < original.max_sessions
                || Instant::now() >= live.expires
                || live.max_bytes < original.max_bytes
                || live.bytes_per_second < original.bytes_per_second
            {
                return Err(RelayError::Unauthorized);
            }
        }
    };
    tokio::select! {
        result = copy => result,
        result = check => result,
        _ = sleep_until(deadline) => Err(RelayError::Unauthorized),
    }
}
