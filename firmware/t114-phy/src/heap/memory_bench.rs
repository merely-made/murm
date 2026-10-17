//! A separate poll boundary keeps fixture frames off the normal main-task stack.
use core::sync::atomic::{AtomicUsize, Ordering};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

static REQUEST: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RESULT: Signal<CriticalSectionRawMutex, [usize; 4]> = Signal::new();
static MIN_SP: AtomicUsize = AtomicUsize::new(usize::MAX);

fn checkpoint() {
    MIN_SP.fetch_min(cortex_m::register::msp::read() as usize, Ordering::Relaxed);
    crate::crash::memory_bench_checkpoint();
}

#[embassy_executor::task]
pub async fn worker() {
    loop {
        REQUEST.wait().await;
        MIN_SP.store(usize::MAX, Ordering::Relaxed);
        let values = protocol_capacity_probe::target_round::run(
            super::used,
            super::bench_refuse_oversize,
            checkpoint,
        );
        protocol_capacity_probe::runtime_pressure::run(checkpoint);
        assert_eq!(
            super::used(),
            values[3],
            "runtime pressure leaked allocations"
        );
        RESULT.signal(values);
    }
}

/// The sole command handler waits for completion before accepting another request.
pub async fn measure() -> [usize; 4] {
    REQUEST.signal(());
    RESULT.wait().await
}

/// Lowest MSP at explicit fixture checkpoints. Deeper callees/IRQs are unmeasured.
pub fn sampled_min_sp() -> usize {
    MIN_SP.load(Ordering::Relaxed)
}
