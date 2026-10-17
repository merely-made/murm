//! Before-radio flash work runs after the large main poll frame has unwound.
//! Both turns share the same boot-painted MSP. The sole flash owner is moved
//! through the signals and returned before any radio driver is constructed.
use crate::store::SettingsStore;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
static REQUEST: Signal<CriticalSectionRawMutex, SettingsStore<'static>> = Signal::new();
static RESULT: Signal<CriticalSectionRawMutex, SettingsStore<'static>> = Signal::new();
#[embassy_executor::task]
async fn worker() {
    let mut store = REQUEST.wait().await;
    crate::contact_bench::run(&mut store);
    RESULT.signal(store);
}
pub async fn run(store: SettingsStore<'static>, spawner: Spawner) -> SettingsStore<'static> {
    spawner.spawn(worker().expect("one before-radio contact fixture"));
    REQUEST.signal(store);
    RESULT.wait().await
}
