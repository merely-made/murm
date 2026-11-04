// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
use tokio::time::{Instant, timeout};

fn flow() -> FlowLimits {
    FlowLimits {
        max_sessions: 2,
        max_bytes: 1024,
        bytes_per_second: 1024,
        buffer_bytes: 16,
        lifetime: Duration::from_secs(3),
        authority_interval: Duration::from_millis(5),
    }
}
fn device(sessions: usize, bytes: u64) -> DeviceBudget {
    DeviceBudget::new(DeviceLimits {
        max_pending: 4,
        max_sessions: sessions,
        max_bytes: bytes,
        bytes_per_second: 1024,
    })
    .unwrap()
}
fn grant() -> RelayGrant {
    RelayGrant {
        revision: [1; 32],
        max_sessions: 2,
        max_bytes: 1024,
        bytes_per_second: 1024,
        expires: Instant::now() + Duration::from_secs(3),
    }
}

#[tokio::test]
async fn shared_device_limits_apply_to_independent_pool_reservations() {
    let budget = device(1, 10);
    let first_pool = budget.reserve(flow()).unwrap();
    assert!(matches!(
        budget.clone().reserve(flow()),
        Err(RelayError::Capacity)
    ));
    first_pool.charge(6).await.unwrap();
    drop(first_pool);
    let second_pool = budget.reserve(flow()).unwrap();
    assert!(matches!(
        second_pool.charge(5).await,
        Err(RelayError::Capacity)
    ));
    assert_eq!(budget.stats().charged_bytes, 6);
    drop(second_pool);
    assert_eq!(budget.stats().active, 0);
    assert_eq!(budget.stats().peak_active, 1);
}

#[tokio::test]
async fn both_directions_share_one_cumulative_allowance() {
    let budget = device(1, 10);
    let reservation = budget.reserve(flow()).unwrap();
    let (mut client, a) = duplex(32);
    let (b, mut recipient) = duplex(32);
    let task = tokio::spawn(async move {
        let allowed = grant();
        super::pump::forward(a, b, &reservation, allowed, || Some(allowed)).await
    });
    client.write_all(b"abcdef").await.unwrap();
    let mut first = [0; 6];
    recipient.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"abcdef");
    recipient.write_all(b"12345").await.unwrap();
    assert!(matches!(task.await.unwrap(), Err(RelayError::Capacity)));
    assert_eq!(budget.stats().charged_bytes, 6);
    assert_eq!(budget.stats().active, 0);
}

#[tokio::test]
async fn revoked_slow_reader_releases_the_device_slot() {
    let budget = device(1, 1024);
    let reservation = budget.reserve(flow()).unwrap();
    let (mut client, a) = duplex(16);
    let (b, _slow_reader) = duplex(1);
    let live = Arc::new(AtomicBool::new(true));
    let control = live.clone();
    let task = tokio::spawn(async move {
        let allowed = grant();
        super::pump::forward(a, b, &reservation, allowed, || {
            live.load(Ordering::SeqCst).then_some(allowed)
        })
        .await
    });
    client.write_all(&[3; 16]).await.unwrap();
    control.store(false, Ordering::SeqCst);
    assert!(matches!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap(),
        Err(RelayError::Unauthorized)
    ));
    assert_eq!(budget.stats().active, 0);
    assert!(budget.reserve(flow()).is_ok());
}

#[tokio::test]
async fn cancellation_and_expiry_do_not_leak_capacity() {
    let budget = device(1, 1024);
    let reservation = budget.reserve(flow()).unwrap();
    let (_client, a) = duplex(1);
    let (b, _recipient) = duplex(1);
    let task = tokio::spawn(async move {
        let mut allowed = grant();
        allowed.expires = Instant::now() + Duration::from_millis(10);
        super::pump::forward(a, b, &reservation, allowed, || Some(allowed)).await
    });
    assert!(matches!(task.await.unwrap(), Err(RelayError::Unauthorized)));
    assert_eq!(budget.stats().active, 0);
    let reservation = budget.reserve(flow()).unwrap();
    let task = tokio::spawn(async move {
        let _reservation = reservation;
        std::future::pending::<()>().await;
    });
    task.abort();
    let _ = task.await;
    assert_eq!(budget.stats().active, 0);
}

#[tokio::test]
async fn rate_budget_is_shared_and_waiting_is_cancellable() {
    let budget = DeviceBudget::new(DeviceLimits {
        max_pending: 2,
        max_sessions: 2,
        max_bytes: 1024,
        bytes_per_second: 16,
    })
    .unwrap();
    let first = budget.reserve(flow()).unwrap();
    let second = budget.reserve(flow()).unwrap();
    first.charge(16).await.unwrap();
    assert!(
        timeout(Duration::from_millis(15), second.charge(1))
            .await
            .is_err()
    );
    assert_eq!(budget.stats().charged_bytes, 16);
    drop(first);
    drop(second);
    assert_eq!(budget.stats().active, 0);
}

#[tokio::test]
async fn zero_contribution_disables_reservations() {
    assert!(matches!(
        device(0, 1024).reserve(flow()),
        Err(RelayError::Capacity)
    ));
    assert!(matches!(
        device(1, 0).reserve(flow()),
        Err(RelayError::Capacity)
    ));
    let budget = DeviceBudget::new(DeviceLimits {
        max_pending: 0,
        max_sessions: 1,
        max_bytes: 1024,
        bytes_per_second: 0,
    })
    .unwrap();
    assert!(matches!(budget.reserve(flow()), Err(RelayError::Capacity)));
}
