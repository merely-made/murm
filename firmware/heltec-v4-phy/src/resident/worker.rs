//! Pure construction on another executor turn, with no flash/radio capability.
//! The USB owner holds its cancel-safe quiet guard while awaiting the result.
//! Boxing the returned runtime avoids another large inline copy in that task.
extern crate alloc;
use super::{
    BuildError, ResidentState,
    build::{self, Prepared},
};
use alloc::boxed::Box;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

static REQUEST: Signal<CriticalSectionRawMutex, Prepared> = Signal::new();
static RESULT: Signal<CriticalSectionRawMutex, Result<Box<ResidentState>, BuildError>> =
    Signal::new();

#[embassy_executor::task]
async fn builder() {
    let prepared = REQUEST.wait().await;
    RESULT.signal(build::build(prepared).map(Box::new));
}
pub(crate) fn start_builder(spawner: Spawner) {
    spawner.spawn(builder().expect("one resident construction per boot"));
}
pub(crate) async fn construct(prepared: Prepared) -> Result<Box<ResidentState>, BuildError> {
    REQUEST.signal(prepared);
    RESULT.wait().await
}
