//! A separate poll boundary keeps fixture frames off the normal main-task stack.
extern crate alloc;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicUsize, Ordering};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

static REQUEST: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RESULT: Signal<CriticalSectionRawMutex, [usize; 4]> = Signal::new();
static MIN_SP: AtomicUsize = AtomicUsize::new(usize::MAX);

// Same thread-mode executor as esp_rtos::main, with one early paint hook.
#[esp_hal::main]
fn bench_entry() -> ! {
    crate::stack_watermark::paint_before_executor();
    static EXECUTOR: static_cell::StaticCell<esp_rtos::embassy::Executor> =
        static_cell::StaticCell::new();
    EXECUTOR
        .init(esp_rtos::embassy::Executor::new())
        .run(|spawner| {
            spawner.spawn(crate::main(spawner).expect("one main task"));
        })
}

fn checkpoint() {
    MIN_SP.fetch_min(super::sampled_stack_pointer(), Ordering::Relaxed);
}

#[embassy_executor::task]
pub async fn worker() {
    loop {
        REQUEST.wait().await;
        MIN_SP.store(usize::MAX, Ordering::Relaxed);
        let initial = super::allocator_used();
        // The fixture owns the inline protocol cores. Production additionally
        // boxes that storage, so charge its exact layout during every pressure
        // phase without constructing a second set of protocol state.
        let boxed_storage = Box::<crate::resident::ResidentState>::new_uninit();
        let mut values = protocol_capacity_probe::target_round::run(
            super::allocator_used,
            super::bench_refuse_oversize,
            checkpoint,
        );
        core::hint::black_box(&boxed_storage);
        drop(boxed_storage);
        values[0] = initial;
        values[3] = super::allocator_used();
        RESULT.signal(values);
    }
}

pub const fn boxed_state_bytes() -> usize {
    core::mem::size_of::<crate::resident::ResidentState>()
}

/// The sole command handler waits for completion before accepting another request.
pub async fn measure() -> [usize; 4] {
    REQUEST.signal(());
    RESULT.wait().await
}

/// Lowest a1 at explicit fixture checkpoints. Deeper callees/IRQs are unmeasured.
pub fn sampled_min_sp() -> usize {
    MIN_SP.load(Ordering::Relaxed)
}
