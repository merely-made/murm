//! Explicitly isolated physical qualification, never an identity fallback.
//! The attended bench supplies fresh private material and separate public
//! journals. Missing material is refused; this module never generates a seed.
use super::*;
use radio_hand::instances::{Config, RETINUE, SENNET};
use selvage::personality::{ControllerConfig, CoveragePolicy, InstalledPersonalitySet};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    bench_only: bool,
    port: PathBuf,
    realm: String,
    journal: PathBuf,
    private_material: PathBuf,
    name: String,
    frequency_hz: u32,
    bandwidth_hz: u32,
    spreading_factor: u8,
    coding_rate_denominator: u8,
    tx_power_dbm: i8,
    tx_budget_ms: u64,
    frame_ttl_ms: u64,
    retry_after_ms: u64,
    airtime_window_ms: u64,
    airtime_allowance_ms: u64,
    receive_wait_ms: u64,
    open_settle_ms: u64,
    online_timeout_ms: u64,
    transmit_timeout_ms: u64,
    dtr: bool,
}
struct Authority {
    material: PathBuf,
    realm: String,
    tx_budget_ms: u64,
    frame_ttl_ms: u64,
    retry_after_ms: u64,
}
pub fn settings_from_env() -> Result<Option<ResidentSettings>, String> {
    std::env::var_os("SIGNALMAN_TUCKET_BENCH_CONFIG")
        .map(|path| load(&PathBuf::from(path)))
        .transpose()
}
pub fn load(path: &std::path::Path) -> Result<ResidentSettings, String> {
    let config: ConfigFile =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if !config.bench_only || !config.realm.starts_with("bench-") {
        return Err("Tucket bench requires bench_only=true and a separate bench- realm".into());
    }
    if !config.private_material.is_absolute() || !config.journal.is_absolute() {
        return Err("Tucket bench requires explicit absolute material and journal paths".into());
    }
    if config.tx_budget_ms == 0
        || config.tx_budget_ms >= config.frame_ttl_ms
        || config.retry_after_ms == 0
        || config.online_timeout_ms == 0
        || config.transmit_timeout_ms == 0
    {
        return Err(
            "Tucket bench timing bounds must be nonzero and TX must fit its frame lifetime".into(),
        );
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
            material: config.private_material,
            realm: config.realm,
            tx_budget_ms: config.tx_budget_ms,
            frame_ttl_ms: config.frame_ttl_ms,
            retry_after_ms: config.retry_after_ms,
        }),
    };
    validate(&settings)?;
    Ok(settings)
}
impl ResidentAuthority for Authority {
    fn check(&self) -> Result<(), String> {
        let metadata = std::fs::metadata(&self.material)
            .map_err(|e| format!("bench identity unavailable: {e}"))?;
        if !metadata.is_file() || metadata.len() != 116 {
            return Err("invalid isolated bench material length".into());
        }
        Ok(())
    }
    fn prepare(&self, settings: &ResidentSettings) -> Result<PreparedResident, String> {
        if settings.realm != self.realm {
            return Err("bench identity cannot enter another realm".into());
        }
        let material =
            zeroize::Zeroizing::new(std::fs::read(&self.material).map_err(|e| e.to_string())?);
        if material.len() != 116 {
            return Err("invalid isolated bench material length".into());
        }
        let scope = RadioScope {
            realm: self.realm.clone(),
            meshtastic_node: u32::from_le_bytes(material[112..116].try_into().unwrap()),
            meshtastic_channel: 8,
            meshcore_key: tucket::identity::LocalIdentity::from_seed(
                material[64..96].try_into().unwrap(),
            )
            .identity()
            .pub_key,
        };
        let profile = settings.profile;
        let tx_budget_ms = self.tx_budget_ms;
        let frame_ttl_ms = self.frame_ttl_ms;
        let retry_after_ms = self.retry_after_ms;
        let node_number = scope.meshtastic_node;
        Ok(PreparedResident {
            hold: None,
            authorize: Box::new(|| Ok(())),
            can_transmit: Box::new(|| Ok(())),
            scope,
            build: Box::new(move |now| {
                let node = retinue::node::Node::<8, 4, 1, 4>::new(
                    retinue::identity::PrivateIdentity::from_secret_bytes(
                        material[..64].try_into().unwrap(),
                    ),
                    retinue::destination::DestinationName::new("retinue", ["signalman-bench"])
                        .name_hash(),
                );
                // The generic runtime retains all three instances. Only Tucket is
                // activated or projected. This is not a production Sennet lease.
                let sennet = sennet::instance::SennetInstance::new(
                    sennet::instance::SennetInstanceConfig {
                        channel: sennet::node::Channel {
                            hash: 8,
                            key: sennet::transport::ChannelKey::Aes128(
                                material[96..112].try_into().unwrap(),
                            ),
                        },
                        flood: sennet::flood::ManagedFloodConfig {
                            channel_hash: 8,
                            relay_node: 1,
                            seen_capacity: 16,
                            delay: sennet::flood::RelayDelayWindow::new(
                                Duration::ZERO,
                                Duration::ZERO,
                            )
                            .unwrap(),
                        },
                        directory: Default::default(),
                        pending_ttl: frame_ttl_ms,
                    },
                    sennet::instance::PacketIdLease::new(node_number, 1, 65, 1)
                        .map_err(|e| format!("{e:?}"))?,
                )
                .map_err(|e| format!("{e:?}"))?;
                let tucket = tucket::instance::Instance::new(
                    tucket::node::Node::new(
                        tucket::identity::LocalIdentity::from_seed(
                            material[64..96].try_into().unwrap(),
                        ),
                        false,
                    ),
                    tucket::instance::InstanceConfig::new(8, retry_after_ms)
                        .ok_or("invalid Tucket bench retry bounds")?,
                );
                Runtime::new(
                    now,
                    Config {
                        controller: ControllerConfig {
                            home: TUCKET,
                            pin: Some(TUCKET),
                            installed: InstalledPersonalitySet::new(&[RETINUE, SENNET, TUCKET])
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
                    node,
                    sennet,
                    tucket,
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
    fn explicit_bench_material_is_required_and_identity_survives_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let material_path = dir.path().join("private.bin");
        let mut config = serde_json::json!({
            "bench_only": true, "port": "unused-test-port", "realm": "bench-test",
            "journal": dir.path().join("public.json"), "private_material": material_path,
            "name": "Tucket bench", "frequency_hz": 906875000, "bandwidth_hz": 250000,
            "spreading_factor": 10, "coding_rate_denominator": 5, "tx_power_dbm": 0,
            "tx_budget_ms": 2000, "frame_ttl_ms": 15000, "retry_after_ms": 3000,
            "airtime_window_ms": 60000, "airtime_allowance_ms": 6000,
            "receive_wait_ms": 50, "open_settle_ms": 800, "online_timeout_ms": 3000,
            "transmit_timeout_ms": 3000, "dtr": false,
        });
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let settings = load(&config_path).unwrap();
        assert!(settings.authority.check().is_err());
        assert!(settings.authority.prepare(&settings).is_err());
        assert!(!material_path.exists());
        std::fs::write(&material_path, [0x52; 116]).unwrap();
        let mut previous = None;
        for _ in 0..2 {
            let prepared = settings.authority.prepare(&settings).unwrap();
            let runtime = (prepared.build)(0).unwrap();
            assert_eq!(runtime.active().unwrap().instance, TUCKET);
            assert_eq!(
                runtime.tucket().node().identity().pub_key,
                prepared.scope.meshcore_key
            );
            if let Some(previous) = previous {
                assert_eq!(prepared.scope, previous);
            }
            previous = Some(prepared.scope);
        }
        let mut other = settings.clone();
        other.realm = "production".into();
        assert!(settings.authority.prepare(&other).is_err());
        config["bench_only"] = false.into();
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(load(&config_path).is_err());
    }
}
