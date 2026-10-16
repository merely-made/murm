//! Exercise the actual board adapter without linking ESP hardware drivers.
#![forbid(unsafe_code)]

#[path = "../../../firmware/heltec-v4-phy/src/host/boot_rx.rs"]
mod boot_rx;

use boot_rx::BootRx;
use embedded_io_async::{ErrorType, Read};
use std::{
    collections::VecDeque,
    convert::Infallible,
    future::Future,
    task::{Context, Poll, Waker},
};

struct Input {
    chunks: VecDeque<Vec<u8>>,
    pending_at_end: bool,
}

impl Input {
    fn new(chunks: &[&[u8]]) -> Self {
        Self {
            chunks: chunks.iter().map(|bytes| bytes.to_vec()).collect(),
            pending_at_end: false,
        }
    }
}

impl ErrorType for Input {
    type Error = Infallible;
}
impl Read for Input {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Infallible> {
        let Some(mut chunk) = self.chunks.pop_front() else {
            return if self.pending_at_end {
                std::future::pending().await
            } else {
                Ok(0)
            };
        };
        let count = out.len().min(chunk.len());
        out[..count].copy_from_slice(&chunk[..count]);
        chunk.drain(..count);
        if !chunk.is_empty() {
            self.chunks.push_front(chunk);
        }
        Ok(count)
    }
}

#[tokio::test]
async fn permitted_read_only_queries_select_ordinary_service() {
    for name in ["status", "name", "region", "channel", "sync", "ui"] {
        for ending in ["\n", "\r\n"] {
            let bytes = format!("{name}{ending}").into_bytes();
            let mut rx = BootRx::new(Input::new(&[&bytes]));
            assert!(rx.prefetch_query(true).await.unwrap());
            let mut read = [0; 64];
            let count = rx.read(&mut read).await.unwrap();
            assert_eq!(&read[..count], &bytes);
        }
    }
}

#[tokio::test]
async fn pending_or_unreadable_state_cannot_bypass_physical_presence() {
    let mut rx = BootRx::new(Input::new(&[b"status\n"]));
    assert!(!rx.prefetch_query(false).await.unwrap());
    let mut read = [0; 64];
    let count = rx.read(&mut read).await.unwrap();
    assert_eq!(&read[..count], b"status\n");
}

#[tokio::test]
async fn mutations_binary_frames_and_unknown_data_remain_for_the_real_parser() {
    for bytes in [
        b"name set SB2\n".as_slice(),
        b"channel modem\n",
        b"region us915\n",
        b"?\n",
        &[0xc0, 0x51, 0xdb, 0xdc, 0xc0],
        b"status\nextra",
    ] {
        let mut rx = BootRx::new(Input::new(&[bytes, b"tail"]));
        assert!(!rx.prefetch_query(true).await.unwrap());
        let mut restored = Vec::new();
        let mut read = [0; 2];
        loop {
            let count = rx.read(&mut read).await.unwrap();
            if count == 0 {
                break;
            }
            restored.extend_from_slice(&read[..count]);
        }
        assert_eq!(restored, [bytes, b"tail"].concat());
    }
}

#[tokio::test]
async fn fragmented_query_is_recognized_and_replayed_once_before_live_data() {
    let mut rx = BootRx::new(Input::new(&[b"sta", b"tus\r", b"\n", b"live"]));
    assert!(rx.prefetch_query(true).await.unwrap());
    let mut read = [0; 64];
    let count = rx.read(&mut read).await.unwrap();
    assert_eq!(&read[..count], b"status\r\n");
    let count = rx.read(&mut read).await.unwrap();
    assert_eq!(&read[..count], b"live");
}

#[tokio::test]
async fn gesture_winning_during_prefetch_keeps_every_received_byte() {
    let mut input = Input::new(&[b"sta"]);
    input.pending_at_end = true;
    let mut rx = BootRx::new(input);
    {
        let mut future = std::pin::pin!(rx.prefetch_query(true));
        let mut context = Context::from_waker(Waker::noop());
        assert_eq!(future.as_mut().poll(&mut context), Poll::Pending);
    }
    let mut read = [0; 64];
    let count = rx.read(&mut read).await.unwrap();
    assert_eq!(&read[..count], b"sta");
}
