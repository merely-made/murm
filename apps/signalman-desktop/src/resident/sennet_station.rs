//! Djinn-scoped Sennet node and host-durable nonce lease. The first bounded
//! channel lane selects the bench's public LongFast key explicitly; private
//! channel provisioning and authenticated senders are not implied.
use super::*;
use mere_signalman::DjinnStationAuthority;
use radio_hand::instances::Config;
use selvage::personality::{ControllerConfig, CoveragePolicy, InstalledPersonalitySet};
use serde::Deserialize;
use signalman::radio_comms::sennet_ids::SennetPacketIds;

const PUBLIC_LONGFAST: [u8; 16] = [
    0xd4, 0xf1, 0xbb, 0x3a, 0x20, 0x29, 0x07, 0x59, 0xf0, 0xbc, 0xff, 0xab, 0xcf, 0x4e, 0x69, 0x01,
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    directed: Option<DirectedConfig>,
    device_id: pandect::DeviceId,
    port: PathBuf,
    realm: String,
    journal: PathBuf,
    packet_ids: PathBuf,
    name: String,
    public_channel: String,
    peer_node: u32,
    frequency_hz: u32,
    tx_power_dbm: i8,
    tx_budget_ms: u64,
    frame_ttl_ms: u64,
    reservation_size: u32,
    airtime_window_ms: u64,
    airtime_allowance_ms: u64,
    receive_wait_ms: u64,
    open_settle_ms: u64,
    online_timeout_ms: u64,
    transmit_timeout_ms: u64,
    dtr: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectedConfig {
    peer_public_key: String,
    ack_timeout_ms: u64,
    seen_capacity: usize,
}
fn directed_key(config: &DirectedConfig) -> Result<[u8; 32], String> {
    let key: [u8; 32] = hex::decode(&config.peer_public_key)
        .map_err(|_| "invalid peer public key")?
        .try_into()
        .map_err(|_| "require full peer public key")?;
    if config.ack_timeout_ms == 0
        || config.seen_capacity == 0
        || config.seen_capacity > 4096
        || key == [0; 32]
    {
        return Err("directed Sennet requires a pinned peer public key, response window and bounded seen capacity".into());
    }
    Ok(key)
}
struct Authority {
    config: ConfigFile,
    djinn: DjinnStationAuthority,
}
fn parse(path: &std::path::Path) -> Result<ConfigFile, String> {
    let config: ConfigFile =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    personae::sennet::station_salt(config.device_id.as_uuid().as_bytes(), &config.realm)
        .map_err(|e| e.to_string())?;
    if let Some(directed) = &config.directed {
        directed_key(directed)?;
    }
    if config.public_channel != "LongFast"
        || !config.journal.is_absolute()
        || !config.packet_ids.is_absolute()
        || config.packet_ids == config.journal
        || config.reservation_size == 0
        || config.reservation_size > 65536
        || config.tx_budget_ms == 0
        || config.tx_budget_ms >= config.frame_ttl_ms
        || config.online_timeout_ms == 0
        || config.transmit_timeout_ms == 0
        || matches!(config.peer_node, 0 | u32::MAX)
    {
        return Err("Sennet requires public LongFast, distinct absolute journal/packet-ID paths, an identified peer and bounded timing/reservations".into());
    }
    Ok(config)
}
/// Read public-only activation configuration. This opens no wallet or radio.
pub fn load(path: &std::path::Path) -> Result<ResidentSettings, String> {
    let config = parse(path)?;
    let mut profile = PhyProfile::meshtastic_long_fast(config.frequency_hz);
    profile.tx_power_dbm = config.tx_power_dbm;
    let settings = ResidentSettings {
        protocol: signalman::radio_comms::companion::CompanionProtocol::Meshtastic { channel: 0 },
        port: config.port.clone(),
        realm: config.realm.clone(),
        journal: config.journal.clone(),
        name: config.name.clone(),
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
            config,
            djinn: DjinnStationAuthority::default(),
        }),
    };
    validate(&settings)?;
    Ok(settings)
}
/// Explicit first commissioning, or a public-only identity receipt. Connection
/// startup never initializes missing nonce state.
pub fn station_card(path: &std::path::Path, initialize: bool) -> Result<serde_json::Value, String> {
    let config = parse(path)?;
    let released = DjinnStationAuthority::default()
        .open_sennet(config.device_id, &config.realm)
        .map_err(|e| e.to_string())?;
    let source = released.identity.node_number();
    if source == config.peer_node {
        return Err("Sennet public node number collides with the identified stock peer".into());
    }
    released.lease.check().map_err(|e| e.to_string())?;
    if initialize {
        SennetPacketIds::initialize(&config.packet_ids, &config.realm, source)
            .map_err(|e| e.to_string())?;
    }
    released.lease.check().map_err(|e| e.to_string())?;
    let pki = sennet::pki::LocalIdentity::from_secret(*released.material.seed_bytes());
    Ok(
        serde_json::json!({"schema":1,"authority":"djinn","device_id":config.device_id,"realm":config.realm,"node_number":source,"address":format!("{source:08x}"),"pki_public_key":hex::encode(pki.public_key()),"public_channel":"LongFast","channel_hash":8,"packet_ids_initialized":initialize}),
    )
}
impl ResidentAuthority for Authority {
    fn check(&self) -> Result<(), String> {
        self.djinn
            .sennet_wallet()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn prepare(&self, settings: &ResidentSettings) -> Result<PreparedResident, String> {
        let config = &self.config;
        if settings.realm != config.realm {
            return Err("Sennet authority cannot enter another realm".into());
        }
        let released = self
            .djinn
            .open_sennet(config.device_id, &config.realm)
            .map_err(|e| e.to_string())?;
        let source = released.identity.node_number();
        if source == config.peer_node {
            return Err("Sennet public node number collides with the identified stock peer".into());
        }
        let directed = if let Some(d) = &config.directed {
            let identity = sennet::pki::LocalIdentity::from_secret(*released.material.seed_bytes());
            let public = directed_key(d)?;
            identity
                .peer(source, config.peer_node, public)
                .map_err(|e| format!("invalid pinned PKI peer: {e:?}"))?;
            Some((identity, public, d.ack_timeout_ms, d.seen_capacity))
        } else {
            None
        };
        let (reservation, ids) = SennetPacketIds::reserve(
            &config.packet_ids,
            &config.realm,
            source,
            config.reservation_size,
        )
        .map_err(|e| e.to_string())?;
        let lease = Arc::new(released.lease);
        let live = lease.clone();
        let profile = settings.profile;
        let peer_node = config.peer_node;
        let tx_budget_ms = config.tx_budget_ms;
        let frame_ttl_ms = config.frame_ttl_ms;
        Ok(PreparedResident {
            scope: RadioScope {
                realm: config.realm.clone(),
                meshtastic_node: source,
                meshtastic_channel: 8,
                meshcore_key: [0; 32],
            },
            hold: Some(Box::new(reservation)),
            authorize: Box::new(move || lease.check().map_err(|e| e.to_string())),
            can_transmit: Box::new(move || live.check_live().map_err(|e| e.to_string())),
            build: Box::new(move |now| {
                let mut instance = sennet::instance::SennetInstance::new(
                    sennet::instance::SennetInstanceConfig {
                        channel: sennet::node::Channel {
                            hash: 8,
                            key: sennet::transport::ChannelKey::Aes128(PUBLIC_LONGFAST),
                        },
                        flood: sennet::flood::ManagedFloodConfig {
                            channel_hash: 8,
                            relay_node: source as u8,
                            seen_capacity: 64,
                            delay: sennet::flood::RelayDelayWindow::new(
                                Duration::ZERO,
                                Duration::ZERO,
                            )
                            .unwrap(),
                        },
                        directory: Default::default(),
                        pending_ttl: frame_ttl_ms,
                    },
                    ids,
                )
                .map_err(|e| format!("{e:?}"))?;
                if let Some((identity, public, timeout, capacity)) = directed {
                    instance
                        .install_pki(peer_node, identity, public, timeout, capacity)
                        .map_err(|e| format!("{e:?}"))?;
                }
                Runtime::sennet_only(
                    now,
                    Config {
                        controller: ControllerConfig {
                            home: SENNET,
                            pin: Some(SENNET),
                            installed: InstalledPersonalitySet::new(&[SENNET])
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
                    instance,
                )
                .map_err(|e| format!("{e:?}"))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(dir: &std::path::Path) -> serde_json::Value {
        serde_json::json!({"device_id":pandect::DeviceId::new(),"port":dir.join("no-radio"),
            "realm":"bench-sennet-absent","journal":dir.join("journal.json"),"packet_ids":dir.join("ids.json"),
            "name":"Sennet bench","public_channel":"LongFast","peer_node":7,"frequency_hz":906875000,
            "tx_power_dbm":0,"tx_budget_ms":2000,"frame_ttl_ms":15000,"reservation_size":256,
            "airtime_window_ms":60000,"airtime_allowance_ms":12000,"receive_wait_ms":50,
            "open_settle_ms":800,"online_timeout_ms":3000,"transmit_timeout_ms":3000,"dtr":false})
    }
    #[test]
    fn directed_config_requires_public_key_window_and_bounded_dedup_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut value = config(dir.path());
        let public = hex::encode(sennet::pki::LocalIdentity::from_secret([7; 32]).public_key());
        value["directed"] =
            serde_json::json!({"peer_public_key":public,"ack_timeout_ms":15000,"seen_capacity":64});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_ok());
        for (field, bad) in [
            ("peer_public_key", serde_json::json!("00".repeat(32))),
            ("peer_public_key", serde_json::json!("0123")),
            ("ack_timeout_ms", serde_json::json!(0)),
            ("seen_capacity", serde_json::json!(0)),
            ("seen_capacity", serde_json::json!(4097)),
        ] {
            let mut bad_value = value.clone();
            bad_value["directed"][field] = bad;
            std::fs::write(&path, serde_json::to_vec(&bad_value).unwrap()).unwrap();
            assert!(load(&path).is_err());
        }
        value["directed"]["private_key"] = serde_json::json!("forbidden");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn absent_sennet_custody_never_initializes_nonce_state_or_opens_serial_or_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("public-config.json");
        std::fs::write(&path, serde_json::to_vec(&config(dir.path())).unwrap()).unwrap();
        let mut settings = load(&path).unwrap();
        settings.authority = Arc::new(Authority {
            config: parse(&path).unwrap(),
            djinn: DjinnStationAuthority::at(format!(
                r"\\.\pipe\signalman-sennet-absent-{}",
                pandect::DeviceId::new().as_uuid()
            )),
        });
        let worker = ResidentWorker::spawn(settings, Arc::new(|| {}));
        assert!(matches!(
            worker.events.recv_timeout(Duration::from_secs(10)).unwrap(),
            CompanionEvent::Stopped {
                protocol: ProtocolKind::Meshtastic,
                ..
            }
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn sennet_configuration_refuses_private_fields_and_nonpublic_channel_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut value = config(dir.path());
        value["private_material"] = "forbidden".into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_err());
        value.as_object_mut().unwrap().remove("private_material");
        value["public_channel"] = "private".into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&path).is_err());
    }
}
