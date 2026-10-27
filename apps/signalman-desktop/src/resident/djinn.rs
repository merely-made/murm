//! Djinn-backed Tucket activation. The file is public configuration only;
//! the wallet and all seed persistence remain with the resident.
use super::*;
use station_custody::DjinnStationAuthority;
use radio_hand::instances::Config;
use selvage::personality::{ControllerConfig, CoveragePolicy, InstalledPersonalitySet};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    device_id: pandect::DeviceId,
    port: PathBuf,
    realm: String,
    journal: PathBuf,
    name: String,
    frequency_hz: u32,
    bandwidth_hz: u32,
    spreading_factor: u8,
    coding_rate_denominator: u8,
    tx_power_dbm: i8,
    tx_budget_ms: u64,
    frame_ttl_ms: u64,
    retry_after_ms: u64,
    #[serde(default)]
    reply_delay_ms: Option<u64>,
    airtime_window_ms: u64,
    airtime_allowance_ms: u64,
    receive_wait_ms: u64,
    open_settle_ms: u64,
    online_timeout_ms: u64,
    transmit_timeout_ms: u64,
    dtr: bool,
}

struct Authority {
    djinn: DjinnStationAuthority,
    device: pandect::DeviceId,
    realm: String,
    tx_budget_ms: u64,
    frame_ttl_ms: u64,
    retry_after_ms: u64,
    reply_delay_ms: u64,
}

/// Export a signed public contact card through the same scoped Djinn release.
/// This does not open a radio or persist any private material.
pub fn contact_card(path: &std::path::Path) -> Result<serde_json::Value, String> {
    let settings = load(path)?;
    let config: ConfigFile =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let released = DjinnStationAuthority::default()
        .open_tucket(config.device_id, &config.realm)
        .map_err(|e| e.to_string())?;
    let identity = tucket::identity::LocalIdentity::from_seed(*released.material.seed_bytes());
    let public = identity.identity().pub_key;
    let mut node = tucket::node::Node::new(identity, false);
    let data = tucket::advert::AdvertData::try_chat(&settings.name).ok_or("invalid Tucket name")?;
    let timestamp = u32::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs(),
    )
    .map_err(|e| e.to_string())?;
    released.lease.check().map_err(|e| e.to_string())?;
    let frame = node
        .advert_frame_data(timestamp, &data)
        .ok_or("invalid Tucket advert")?;
    released.lease.check().map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"schema":1,"authority":"djinn","device_id":config.device_id,
        "realm":config.realm,"public_key":hex::encode(public),"advert_hex":hex::encode(frame)}),
    )
}

pub fn load(path: &std::path::Path) -> Result<ResidentSettings, String> {
    let config: ConfigFile =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    personae::tucket::station_salt(config.device_id.as_uuid().as_bytes(), &config.realm)
        .map_err(|e| e.to_string())?;
    let reply_delay_ms = config
        .reply_delay_ms
        .unwrap_or(tucket::instance::DEFAULT_REPLY_DELAY_MS);
    if !config.journal.is_absolute()
        || config.tx_budget_ms == 0
        || config.tx_budget_ms >= config.frame_ttl_ms
        || config.retry_after_ms == 0
        || config.online_timeout_ms == 0
        || config.transmit_timeout_ms == 0
        || reply_delay_ms
            .checked_add(config.tx_budget_ms)
            .is_none_or(|end| end >= config.frame_ttl_ms)
    {
        return Err("Tucket requires an absolute journal and nonzero bounded timing".into());
    }
    let mut profile = PhyProfile::meshcore(
        config.frequency_hz,
        config.bandwidth_hz,
        config.spreading_factor,
        config.coding_rate_denominator,
    );
    profile.tx_power_dbm = config.tx_power_dbm;
    let settings = ResidentSettings {
        protocol: signalman::radio_comms::companion::CompanionProtocol::MeshCore,
        port: config.port,
        realm: config.realm.clone(),
        journal: config.journal,
        name: config.name,
        profile,
        airtime_window_ms: config.airtime_window_ms,
        airtime_allowance_ms: config.airtime_allowance_ms,
        receive_wait: Duration::from_millis(config.receive_wait_ms),
        serial: DirectPhySerialConfig {
            dtr: config.dtr,
            open_settle: Duration::from_millis(config.open_settle_ms),
            online_timeout: Duration::from_millis(config.online_timeout_ms),
            transmit_timeout: Duration::from_millis(config.transmit_timeout_ms),
            ..Default::default()
        },
        authority: Arc::new(Authority {
            djinn: DjinnStationAuthority::default(),
            device: config.device_id,
            realm: config.realm,
            tx_budget_ms: config.tx_budget_ms,
            frame_ttl_ms: config.frame_ttl_ms,
            retry_after_ms: config.retry_after_ms,
            reply_delay_ms,
        }),
    };
    validate(&settings)?;
    Ok(settings)
}

impl ResidentAuthority for Authority {
    fn check(&self) -> Result<(), String> {
        self.djinn
            .tucket_wallet()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn prepare(&self, settings: &ResidentSettings) -> Result<PreparedResident, String> {
        if settings.realm != self.realm {
            return Err("Tucket authority cannot enter another realm".into());
        }
        let released = self
            .djinn
            .open_tucket(self.device, &self.realm)
            .map_err(|e| e.to_string())?;
        let identity = tucket::identity::LocalIdentity::from_seed(*released.material.seed_bytes());
        let scope = RadioScope {
            realm: self.realm.clone(),
            meshtastic_node: 0,
            meshtastic_channel: 0,
            meshcore_key: identity.identity().pub_key,
        };
        let profile = settings.profile;
        let tx_budget_ms = self.tx_budget_ms;
        let frame_ttl_ms = self.frame_ttl_ms;
        let retry_after_ms = self.retry_after_ms;
        let reply_delay_ms = self.reply_delay_ms;
        let lease = Arc::new(released.lease);
        let live = lease.clone();
        Ok(PreparedResident {
            hold: None,
            scope,
            authorize: Box::new(move || lease.check().map_err(|e| e.to_string())),
            can_transmit: Box::new(move || live.check_live().map_err(|e| e.to_string())),
            build: Box::new(move |now| {
                Runtime::tucket_only(
                    now,
                    Config {
                        controller: ControllerConfig {
                            home: TUCKET,
                            pin: Some(TUCKET),
                            installed: InstalledPersonalitySet::new(&[TUCKET])
                                .map_err(|e| format!("{e:?}"))?,
                            coverage: CoveragePolicy::AllowGap,
                            max_excursion_ms: frame_ttl_ms,
                            return_budget_ms: tx_budget_ms,
                            max_defer_ms: tx_budget_ms,
                            transition_timeout_ms: tx_budget_ms,
                        },
                        profiles: [profile; 3],
                        tx_budget_ms,
                        frame_ttl_ms,
                    },
                    tucket::instance::Instance::new(
                        tucket::node::Node::new(identity, false),
                        tucket::instance::InstanceConfig::new(8, retry_after_ms)
                            .ok_or("invalid Tucket retry bounds")?
                            .with_reply_delay_ms(reply_delay_ms),
                    ),
                )
                .map_err(|e| format!("{e:?}"))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_djinn_cannot_open_serial_or_create_local_identity_state() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = format!(
            r"\\.\pipe\signalman-tucket-absent-{}",
            pandect::DeviceId::new().as_uuid()
        );
        let authority = Arc::new(Authority {
            djinn: DjinnStationAuthority::at(endpoint),
            device: pandect::DeviceId::new(),
            realm: "custody-test".into(),
            tx_budget_ms: 2_000,
            frame_ttl_ms: 15_000,
            retry_after_ms: 3_000,
            reply_delay_ms: 200,
        });
        let settings = ResidentSettings {
            protocol: signalman::radio_comms::companion::CompanionProtocol::MeshCore,
            port: dir.path().join("radio-does-not-exist"),
            realm: "custody-test".into(),
            journal: dir.path().join("journal.json"),
            name: "Tucket".into(),
            profile: PhyProfile::meshcore(906_875_000, 250_000, 10, 5),
            airtime_window_ms: 60_000,
            airtime_allowance_ms: 12_000,
            receive_wait: Duration::from_millis(50),
            serial: Default::default(),
            authority,
        };
        let worker = ResidentWorker::spawn(settings, Arc::new(|| {}));
        match worker.events.recv_timeout(Duration::from_secs(10)).unwrap() {
            CompanionEvent::Stopped { detail, .. } => assert!(
                detail.contains("djinn") || detail.contains("custody"),
                "{detail}"
            ),
            _ => panic!("absent custody must never connect"),
        }
        drop(worker);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
