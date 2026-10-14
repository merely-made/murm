//! Stock companion ownership on a worker thread; no UI/GPU work runs here.
use crate::network::LayoutWake;
use signalman::{
    comms_adapter::comms::ProtocolKind,
    radio_comms::{
        FileJournal, RadioComms,
        companion::{CompanionProtocol, CompanionSession},
    },
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, Debug)]
pub struct CompanionSettings {
    pub protocol: CompanionProtocol,
    pub port: PathBuf,
    pub realm: String,
    pub journal: PathBuf,
}
/// Explicit attended activation, like the existing Retinue station. Each
/// protocol has its own identity-bound public journal and host-selected realm.
pub fn settings_from_env() -> Result<Vec<CompanionSettings>, String> {
    let mut result = vec![];
    for (prefix, protocol) in [
        ("MESHTASTIC", CompanionProtocol::Meshtastic { channel: 0 }),
        ("MESHCORE", CompanionProtocol::MeshCore),
    ] {
        let Some(port) = std::env::var_os(format!("SIGNALMAN_{prefix}_PORT")) else {
            continue;
        };
        if port.is_empty() {
            return Err(format!("SIGNALMAN_{prefix}_PORT is empty"));
        }
        let realm = std::env::var(format!("SIGNALMAN_{prefix}_REALM")).map_err(|_| {
            format!("SIGNALMAN_{prefix}_REALM is required: name the configured radio realm")
        })?;
        let journal = std::env::var_os(format!("SIGNALMAN_{prefix}_JOURNAL"))
            .ok_or_else(|| format!("SIGNALMAN_{prefix}_JOURNAL is required"))?;
        let protocol = if prefix == "MESHTASTIC" {
            let channel = std::env::var("SIGNALMAN_MESHTASTIC_CHANNEL")
                .unwrap_or_else(|_| "0".into())
                .parse::<u8>()
                .map_err(|_| "invalid Meshtastic channel index")?;
            if channel > 7 {
                return Err("Meshtastic channel index must be 0..7".into());
            }
            CompanionProtocol::Meshtastic { channel }
        } else {
            protocol
        };
        result.push(CompanionSettings {
            protocol,
            port: port.into(),
            realm,
            journal: journal.into(),
        });
    }
    let station = std::env::var_os("SIGNALMAN_STATION_PORT").map(PathBuf::from);
    validate_settings(&result, station.as_deref())?;
    Ok(result)
}
pub fn validate_settings(
    settings: &[CompanionSettings],
    station: Option<&std::path::Path>,
) -> Result<(), String> {
    for (index, setting) in settings.iter().enumerate() {
        if setting.port.as_os_str().is_empty() || setting.journal.as_os_str().is_empty() {
            return Err("A serial port and public journal path are required".into());
        }
        if setting.realm.is_empty()
            || setting.realm.len() > 128
            || !setting
                .realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Realm must contain 1..128 letters, digits, hyphens or underscores".into());
        }
        if matches!(setting.protocol, CompanionProtocol::Meshtastic { channel } if channel > 7) {
            return Err("Meshtastic channel index must be 0..7".into());
        }
        if settings[..index].iter().any(|other| {
            other.protocol.kind() == setting.protocol.kind()
                || port_key(&other.port) == port_key(&setting.port)
                || port_key(&other.journal) == port_key(&setting.journal)
        }) {
            return Err(
                "Each protocol needs one owner, a distinct serial port and a distinct journal"
                    .into(),
            );
        }
        if station.is_some_and(|station| port_key(station) == port_key(&setting.port)) {
            return Err("Port is already selected for the Retinue station".into());
        }
    }
    Ok(())
}
fn port_key(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .replace("/dev/cu.", "/dev/tty.")
}
pub enum CompanionEvent {
    Connected {
        protocol: ProtocolKind,
        owner: RadioComms,
        stop: Arc<AtomicBool>,
        port: String,
    },
    Progress,
    Stopped {
        protocol: ProtocolKind,
        detail: String,
    },
}
pub struct CompanionWorker {
    pub protocol: ProtocolKind,
    events: Receiver<CompanionEvent>,
    progress: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl CompanionWorker {
    pub fn spawn(settings: CompanionSettings, wake: LayoutWake) -> Self {
        let protocol = settings.protocol.kind();
        let (tx, events) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let actor_stop = stop.clone();
        let progress = Arc::new(AtomicBool::new(false));
        let actor_progress = progress.clone();
        let join = thread::Builder::new().name("signalman-companion".into()).spawn(move || {
            let protocol = settings.protocol.kind();
            let result = (|| -> Result<(), String> {
                let station = std::env::var_os("SIGNALMAN_STATION_PORT").map(PathBuf::from);
                validate_settings(std::slice::from_ref(&settings), station.as_deref())?;

                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                runtime.block_on(async {
                    if actor_stop.load(Ordering::Relaxed) { return Err("Companion connection cancelled".into()); }
                    let port = serial2_tokio::SerialPort::open(&settings.port, 115_200).map_err(|e| e.to_string())?;
                    #[cfg(unix)]
                    let _exclusive = ExclusivePort::claim(&settings.port).map_err(|e| e.to_string())?;
                    // Assert CDC readiness without entering the flasher/reset sequence.
                    port.set_rts(false).map_err(|e| e.to_string())?;
                    port.set_dtr(true).map_err(|e| e.to_string())?;
                    let connection = CompanionSession::connect(port, settings.protocol, settings.realm, FileJournal::new(settings.journal));
                    let mut session = tokio::select! {
                        result = connection => result.map_err(|e| e.to_string())?,
                        _ = async { while !actor_stop.load(Ordering::Relaxed) { tokio::time::sleep(std::time::Duration::from_millis(100)).await; } } => return Err("Companion connection cancelled".into()),
                    };
                    tx.try_send(CompanionEvent::Connected { protocol, owner: session.owner(), stop: actor_stop.clone(), port: settings.port.display().to_string() }).map_err(|e| e.to_string())?;
                    wake();
                    while !actor_stop.load(Ordering::Relaxed) {
                        if session.poll().await.map_err(|e| e.to_string())? {
                            actor_progress.store(true, Ordering::Release);
                            wake();
                        }
                    }
                    Ok::<_,String>(())
                })
            })();
            let detail = match result { Ok(()) => "Companion disconnected. Queued intents remain durable; restart with the same configuration to reconnect.".into(), Err(e) => format!("Companion stopped: {e}. Uncertain commands require explicit resend.") };
            let _ = tx.try_send(CompanionEvent::Stopped { protocol, detail });
            wake();
        }).expect("spawn companion worker");
        Self {
            protocol,
            events,
            progress,
            stop,
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
impl Drop for CompanionWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
/// TIOCEXCL prevents subsequent Unix opens, including a diagnostic collector.
/// The guard outlives the serial session and explicitly clears exclusivity.
#[cfg(unix)]
struct ExclusivePort(std::fs::File);
#[cfg(unix)]
impl ExclusivePort {
    fn claim(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        rustix::termios::ioctl_tiocexcl(&file)?;
        Ok(Self(file))
    }
}
#[cfg(unix)]
impl Drop for ExclusivePort {
    fn drop(&mut self) {
        let _ = rustix::termios::ioctl_tiocnxcl(&self.0);
    }
}
