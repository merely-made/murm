//! Production DOM controls and journal projection, with no serial/GPU execution.
use cambium_genet_winit_host::Harness;
use genet_probe::Selector;
use signalman::{
    comms_adapter::comms::ProtocolKind,
    radio_comms::{CompanionScope, FileJournal, RadioComms},
};
use signalman_desktop::{
    DesktopState, default_catalog_path, root, sheet,
    views::{Child, Logic},
};
type App = Harness<DesktopState, Logic, Child>;
#[test]
fn stock_connection_controls_work_before_any_identity_has_been_opened() {
    let mut h: App = Harness::new(
        sheet(),
        DesktopState::new(&default_catalog_path()),
        root as Logic,
    );
    h.layout_at(1280.0, 1600.0);
    assert!(h.click_on(&Selector::role("button").containing("Messages")));
    assert!(h.click_on(&Selector::role("button").with_attr("data-message-protocol", "Meshtastic")));
    h.with_surfaces(|surfaces| {
        assert!(genet_probe::text_present(
            surfaces,
            "Stock companion serial port"
        ));
        assert!(genet_probe::text_present(surfaces, "Radio realm name"));
        assert!(genet_probe::text_present(
            surfaces,
            "Public message journal path"
        ));
    });
    let dir = tempfile::tempdir().unwrap();
    h.update(|s| {
        s.companion_port = cambium::TextInput::new("/dev/cu.test-stock");
        s.companion_realm = cambium::TextInput::new("test-realm");
        s.companion_journal =
            cambium::TextInput::new(dir.path().join("stock.json").to_str().unwrap());
    });
    assert!(h.click_on(&Selector::role("button").containing("Connect companion")));
    let mut request = None;
    h.update(|s| request = s.take_companion_request());
    assert_eq!(request.unwrap().realm, "test-realm");
}
#[test]
fn stock_history_and_queue_button_share_the_actual_companion_journal() {
    let dir = tempfile::tempdir().unwrap();
    let owner = RadioComms::open_companion(
        CompanionScope::MeshCore {
            realm: "test-realm".into(),
            key: [7; 32],
        },
        FileJournal::new(dir.path().join("stock.json")),
    )
    .unwrap();
    let mut state = DesktopState::new(&default_catalog_path());
    state.companion_comms.push(owner.clone());
    state.select_message_protocol(ProtocolKind::MeshCore);
    state.message_recipient = cambium::TextInput::new(hex::encode([8; 32]));
    state.message_draft = cambium::TextInput::new("through stock companion");
    let mut h: App = Harness::new(sheet(), state, root as Logic);
    h.layout_at(1280.0, 1600.0);
    assert!(h.click_on(&Selector::role("button").containing("Messages")));
    assert!(h.click_on(&Selector::role("button").containing("Queue text")));
    assert_eq!(owner.records().unwrap().len(), 1);
    h.with_surfaces(|surfaces| {
        assert!(genet_probe::text_present(
            surfaces,
            "through stock companion"
        ))
    });
}
