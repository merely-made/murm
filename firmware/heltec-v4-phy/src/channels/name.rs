//! Persist a public device label through the existing verified settings journal.

use radio_hand::{
    link::HostLink,
    settings::{
        Settings,
        name::{self, NameCommand},
    },
};

pub(super) async fn serve<L: HostLink>(
    command: NameCommand,
    settings: Option<Settings>,
    store: &mut crate::store::SettingsStore,
    host: &mut L,
) {
    let mut reboot = false;
    let reply = match (command, settings) {
        (NameCommand::Invalid, _) => radio_face::Text::from_truncated(
            "name invalid: use 1-16 printable ASCII characters\r\n",
        ),
        (NameCommand::Report, _) | (_, None) => name::report(settings),
        (NameCommand::Set(wanted), Some(current)) if wanted == current.device_name => {
            name::report(settings)
        }
        (NameCommand::Set(device_name), Some(current)) => {
            let next = Settings {
                device_name,
                ..current
            };
            if store.save(&next).is_ok() {
                reboot = true;
                radio_face::Text::from_truncated("name saved; rebooting\r\n")
            } else {
                radio_face::Text::from_truncated("name write failed\r\n")
            }
        }
    };
    let _ = host.write_all(reply.as_str().as_bytes()).await;
    if reboot {
        embassy_time::Timer::after_millis(250).await;
        esp_hal::system::software_reset();
    }
}
