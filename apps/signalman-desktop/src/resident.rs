//! One attended Tucket runtime over Tulle. Hosts supply scoped authority;
//! neither the UI nor this worker invents a protocol identity.
use crate::{companion::CompanionEvent, network::LayoutWake};
use radio_hand::instances::{Runtime, TUCKET};
use signalman::{
    comms_adapter::comms::ProtocolKind,
    radio_comms::{FileJournal, RadioComms, RadioScope, carriage::ResidentCarriage},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tulle::{
    PhyProfile,
    airtime::AirtimeBudget,
    direct_phy_serial::{DirectPhySerialConfig, DirectPhySerialLink},
};
pub mod bench;
#[cfg(test)]
mod tests;

/// An authority releases only this connection's scoped material. The builder
/// constructs protocol state after the physical owner confirms its profile.
pub struct PreparedResident {
    pub scope: RadioScope,
    pub build: Box<dyn FnOnce(u64) -> Result<Runtime, String> + Send>,
}
pub trait ResidentAuthority: Send + Sync {
    fn prepare(&self, settings: &ResidentSettings) -> Result<PreparedResident, String>;
    fn check(&self) -> Result<(), String>;
}
#[derive(Clone)]
pub struct ResidentSettings {
    pub port: PathBuf,
    pub realm: String,
    pub journal: PathBuf,
    pub name: String,
    pub profile: PhyProfile,
    pub airtime_window_ms: u64,
    pub airtime_allowance_ms: u64,
    pub receive_wait: Duration,
    pub serial: DirectPhySerialConfig,
    pub authority: Arc<dyn ResidentAuthority>,
}
pub fn validate(settings: &ResidentSettings) -> Result<(), String> {
    crate::companion::validate_settings(
        &[crate::companion::CompanionSettings {
            protocol: signalman::radio_comms::companion::CompanionProtocol::MeshCore,
            port: settings.port.clone(),
            realm: settings.realm.clone(),
            journal: settings.journal.clone(),
        }],
        std::env::var_os("SIGNALMAN_STATION_PORT")
            .as_deref()
            .map(std::path::Path::new),
    )?;
    if settings.name.is_empty() || settings.name.len() > 31 || settings.name.contains('\0') {
        return Err("Tucket name must contain 1..31 UTF-8 bytes without NUL".into());
    }
    if settings.airtime_window_ms == 0
        || settings.airtime_allowance_ms == 0
        || settings.airtime_allowance_ms > settings.airtime_window_ms
    {
        return Err("Tucket airtime allowance must fit its nonzero window".into());
    }
    if settings.receive_wait.is_zero() || settings.receive_wait > Duration::from_secs(1) {
        return Err("Tucket receive wait must be greater than zero and at most one second".into());
    }
    tulle::lora::LoRaParams::try_from(settings.profile).map_err(str::to_owned)?;
    Ok(())
}
pub struct ResidentWorker {
    events: Receiver<CompanionEvent>,
    stop: Arc<AtomicBool>,
    progress: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl ResidentWorker {
    pub fn spawn(settings: ResidentSettings, wake: LayoutWake) -> Self {
        let (tx, events) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let actor_stop = stop.clone();
        let progress = Arc::new(AtomicBool::new(false));
        let actor_progress = progress.clone();
        let join = thread::Builder::new()
            .name("signalman-tucket".into())
            .spawn(move || {
                let result = run(&settings, &actor_stop, &actor_progress, &tx, &wake);
                let detail = match result {
                    Ok(()) => "Tucket disconnected. History remains durable; reconnect uses the same scoped identity.".into(),
                    Err(error) => format!("Tucket stopped: {error}. Uncertain commands require explicit resend."),
                };
                let _ = tx.try_send(CompanionEvent::Stopped {
                    protocol: ProtocolKind::MeshCore, detail,
                });
                wake();
            })
            .expect("spawn Tucket worker");
        Self {
            events,
            stop,
            progress,
            join: Some(join),
        }
    }
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub fn drain(&self) -> Vec<CompanionEvent> {
        let mut events: Vec<_> = self.events.try_iter().collect();
        if self.progress.swap(false, Ordering::AcqRel) {
            events.push(CompanionEvent::Progress);
        }
        events
    }
}
fn run(
    settings: &ResidentSettings,
    stop: &Arc<AtomicBool>,
    progress: &AtomicBool,
    events: &mpsc::SyncSender<CompanionEvent>,
    wake: &LayoutWake,
) -> Result<(), String> {
    validate(settings)?;
    settings.authority.check()?;
    let prepared = settings.authority.prepare(settings)?;
    if prepared.scope.realm != settings.realm {
        return Err("Tucket authority returned another realm".into());
    }
    if stop.load(Ordering::Relaxed) {
        return Err("Tucket connection cancelled".into());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(async {
            let mut radio = DirectPhySerialLink::open(
                &settings.port,
                settings.profile,
                AirtimeBudget::new(settings.airtime_window_ms, settings.airtime_allowance_ms),
                settings.serial.clone(),
            )
            .map_err(|e| e.to_string())?;
            let result = async {
                tokio::select! {
                    result = radio.wait_online() => result.map_err(|e| e.to_string())?,
                    _ = stopped(stop) => return Err("Tucket connection cancelled".into()),
                }
                settings.authority.check()?;
                let epoch = tokio::time::Instant::now();
                let mut runtime = (prepared.build)(0)?;
                if runtime.active().is_none_or(|a| a.instance != TUCKET)
                    || runtime.profile(TUCKET).map_err(|e| format!("{e:?}"))? != settings.profile
                {
                    return Err("Tucket runtime does not match the confirmed radio profile".into());
                }
                let owner = RadioComms::open(prepared.scope, FileJournal::new(&settings.journal))
                    .map_err(|e| e.to_string())?;
                let mut carriage = ResidentCarriage::new(
                    owner.attach(&mut runtime).map_err(|e| e.to_string())?,
                    &mut radio,
                    epoch,
                    0,
                    settings.receive_wait,
                )
                .map_err(|e| e.to_string())?;
                carriage
                    .advertise(&settings.name)
                    .map_err(|e| e.to_string())?;
                events
                    .try_send(CompanionEvent::Connected {
                        protocol: ProtocolKind::MeshCore,
                        owner,
                        stop: stop.clone(),
                        port: settings.port.display().to_string(),
                        source: "Tucket runs on this computer; Tulle owns the radio.",
                    })
                    .map_err(|e| e.to_string())?;
                wake();
                while !stop.load(Ordering::Relaxed) {
                    settings.authority.check()?;
                    if carriage.poll().await.map_err(|e| e.to_string())? {
                        progress.store(true, Ordering::Release);
                        wake();
                    }
                }
                Ok::<_, String>(())
            }
            .await;
            // Shutdown also runs after protocol, custody or journal errors.
            let closed = radio.shutdown().await.map_err(|e| e.to_string());
            result.and(closed)
        })
}
impl Drop for ResidentWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
async fn stopped(stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
