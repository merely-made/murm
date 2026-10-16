use super::*;

struct Denied {
    locked: bool,
    prepared: AtomicBool,
}
impl ResidentAuthority for Denied {
    fn check(&self) -> Result<(), String> {
        if self.locked {
            Err("custody locked".into())
        } else {
            Ok(())
        }
    }
    fn prepare(&self, _: &ResidentSettings) -> Result<PreparedResident, String> {
        self.prepared.store(true, Ordering::Relaxed);
        Err("scoped Tucket identity absent".into())
    }
}
fn settings(dir: &std::path::Path, authority: Arc<dyn ResidentAuthority>) -> ResidentSettings {
    ResidentSettings {
        protocol: signalman::radio_comms::companion::CompanionProtocol::MeshCore,
        port: dir.join("radio-does-not-exist"),
        realm: "test-realm".into(),
        journal: dir.join("journal.json"),
        name: "test".into(),
        profile: PhyProfile::meshcore(906_875_000, 250_000, 10, 5),
        airtime_window_ms: 60_000,
        airtime_allowance_ms: 6_000,
        receive_wait: Duration::from_millis(50),
        serial: DirectPhySerialConfig::default(),
        authority,
    }
}
#[test]
fn locked_or_absent_identity_stops_before_serial_or_journal_access() {
    for protocol in [
        signalman::radio_comms::companion::CompanionProtocol::MeshCore,
        signalman::radio_comms::companion::CompanionProtocol::Meshtastic { channel: 0 },
    ] {
        for locked in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let authority = Arc::new(Denied {
                locked,
                prepared: AtomicBool::new(false),
            });
            let mut settings = settings(dir.path(), authority.clone());
            settings.protocol = protocol;
            let worker = ResidentWorker::spawn(settings.clone(), Arc::new(|| {}));
            let event = worker.events.recv_timeout(Duration::from_secs(5)).unwrap();
            match event {
                CompanionEvent::Stopped { detail, .. } => {
                    assert!(
                        detail.contains(if locked {
                            "custody locked"
                        } else {
                            "scoped Tucket identity absent"
                        }),
                        "{detail}"
                    );
                }
                _ => panic!("denied custody must never publish a connection"),
            }
            assert_eq!(authority.prepared.load(Ordering::Relaxed), !locked);
            assert!(!settings.journal.exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn desktop_requires_configured_authority_before_scheduling_a_connection() {
    let mut state = crate::state::DesktopState::new(&crate::default_catalog_path());
    state.select_message_protocol(ProtocolKind::MeshCore);
    state.request_resident_connection();
    assert!(state.take_resident_request().is_none());
    assert!(!state.resident_connecting);
    assert!(
        state
            .message_notice
            .unwrap()
            .contains("scoped Tucket authority")
    );
}

#[test]
fn tucket_connection_exposes_only_its_commissioned_message_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let owner = RadioComms::open(
        RadioScope {
            realm: "projection-only".into(),
            meshtastic_node: 7,
            meshtastic_channel: 8,
            meshcore_key: [9; 32],
        },
        FileJournal::new(dir.path().join("public.json")),
    )
    .unwrap();
    let mut state = crate::state::DesktopState::new(&crate::default_catalog_path());
    state.apply_companion_event(CompanionEvent::Connected {
        protocol: ProtocolKind::MeshCore,
        owner,
        stop: Arc::new(AtomicBool::new(false)),
        port: "projection-only".into(),
        source: "test",
    });
    assert_eq!(state.comms().unwrap().identities().len(), 1);
    assert!(state.radio_owner(ProtocolKind::MeshCore).is_some());
    assert!(state.radio_owner(ProtocolKind::Meshtastic).is_none());
}

#[test]
fn connecting_tucket_reserves_its_port_protocol_and_journal() {
    let dir = tempfile::tempdir().unwrap();
    for conflict in ["port", "protocol", "journal"] {
        let authority = Arc::new(Denied {
            locked: true,
            prepared: AtomicBool::new(false),
        });
        let mut resident = settings(dir.path(), authority);
        resident.port = "COM6".into();
        let mut state = crate::state::DesktopState::new(&crate::default_catalog_path());
        state.resident_settings = Some(resident.clone());
        state.resident_connecting = true;
        state.select_message_protocol(if conflict == "protocol" {
            ProtocolKind::MeshCore
        } else {
            ProtocolKind::Meshtastic
        });
        state.companion_port =
            cambium::TextInput::new(if conflict == "port" { "COM6" } else { "COM7" });
        state.companion_realm = cambium::TextInput::new("another-realm");
        state.companion_journal = cambium::TextInput::new(if conflict == "journal" {
            resident.journal.to_str().unwrap()
        } else {
            "separate.json"
        });
        state.request_companion_connection();
        assert!(state.take_companion_request().is_none(), "{conflict}");
        assert!(
            state.message_notice.unwrap().contains("distinct"),
            "{conflict}"
        );
    }
}

#[test]
fn active_radio_blocks_observation_and_installer_serial_borrowing() {
    let mut state = crate::state::DesktopState::new(&crate::default_catalog_path());
    state.resident_connecting = true;
    state.request_observation_start();
    assert!(state.take_observation_request().is_none());
    assert!(!state.observation_collecting);
    let mut worker = None;
    crate::flow::perform(
        &mut state,
        crate::state::Request::Rescan,
        &mut worker,
        Arc::new(|| {}),
    );
    assert!(worker.is_none());
    assert!(
        state
            .refusal
            .iter()
            .any(|s| s.contains("Disconnect the active radio"))
    );
}
