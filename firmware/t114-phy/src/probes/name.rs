//! Persist a public device label through the existing verified settings journal.

use super::Outcome;
use radio_hand::{
    executive::Executive,
    link::HostLink,
    settings::{
        Settings,
        name::{self, NameCommand},
    },
};

pub(super) async fn serve<L, RK, DLY>(
    command: NameCommand,
    settings: Option<Settings>,
    exec: &mut Executive<'_, RK, DLY>,
    host: &mut L,
) -> Outcome
where
    L: HostLink,
    RK: lora_phy::mod_traits::RadioKind,
    DLY: lora_phy::DelayNs,
{
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
            if exec.save_settings(&next).is_ok() {
                reboot = true;
                radio_face::Text::from_truncated("name saved; rebooting\r\n")
            } else {
                radio_face::Text::from_truncated("name write failed\r\n")
            }
        }
    };
    let reported = host.write_all(reply.as_str().as_bytes()).await;
    if reboot {
        embassy_time::Timer::after_millis(250).await;
        cortex_m::peripheral::SCB::sys_reset();
    }
    if reported.is_err() {
        Outcome::HostGone
    } else {
        Outcome::Served
    }
}
