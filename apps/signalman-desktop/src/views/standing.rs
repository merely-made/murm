//! A connected stock radio's firmware against the qualified list.

use cambium::{button, el, text};
use signalman::comms_adapter::comms::ProtocolKind;
use signalman::{FirmwareOffer, FirmwareStanding};

use crate::state::DesktopState;

use super::Child;

fn describe(offer: &FirmwareOffer) -> String {
    let source = if offer.download_urls.is_empty() {
        "in the installed catalog".to_string()
    } else {
        format!("from {}", offer.download_urls.join(", "))
    };
    format!(
        "{} {}, {} bytes, SHA-256 {}, {source}",
        offer.publisher,
        offer.version,
        offer.byte_length,
        offer.sha256.join(", ")
    )
}

/// Listed, newer available, not listed, or unknown; nothing installs from here.
pub(super) fn companion_standing(state: &DesktopState, protocol: ProtocolKind) -> Child {
    let Some(standing) = state.companion_standing(protocol) else {
        return Box::new(el("div", ()).attr("class", "empty-none"));
    };
    let (kind, line, newer) = match standing {
        FirmwareStanding::Listed {
            current,
            newer: None,
        } => (
            "listed",
            format!(
                "Firmware {} is on the qualified list; no newer qualified release.",
                current.version
            ),
            None,
        ),
        FirmwareStanding::Listed {
            current,
            newer: Some(newer),
        } => (
            "newer",
            format!(
                "Firmware {} is on the qualified list. A newer qualified release is available: {}.",
                current.version,
                describe(&newer)
            ),
            Some(newer.package_id),
        ),
        FirmwareStanding::NotListed { reported } => (
            "not-listed",
            format!("Firmware {reported} is not on the qualified list; nothing is offered."),
            None,
        ),
        FirmwareStanding::Unknown { reason } => {
            ("unknown", format!("Firmware not compared: {reason}."), None)
        }
    };
    let choose: Child = match newer {
        Some(package_id) => Box::new(button(
            "Choose this release to review",
            move |s: &mut DesktopState, _| s.choose_package_id(&package_id),
        )),
        None => Box::new(el("div", ()).attr("class", "empty-none")),
    };
    Box::new(
        el("div", (el("div", text(line)), choose))
            .attr("role", "status")
            .attr("data-firmware-standing", kind),
    )
}
