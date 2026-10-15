//! The carrier-independent `[[interface]]` keys (`Reticulum.py` 843-1060).

use core::time::Duration;

use crate::announce_admission::AnnounceIngressPolicy;
use crate::ifac::Ifac;
use crate::node::InterfaceMode;

use super::super::iface_policy::{AnnounceRate, IfacePolicy, PathRequestPolicy};
use super::super::interface::InterfaceId;
use super::super::runtime::Endpoint;
use super::{Keys, RnsConfigError};

/// One interface's RNS settings, as `_synthesize_interface` reads them. A key RNS ignores
/// when out of range (a bitrate below 5, an announce cap outside (0, 100], an
/// `ifac_size` below 8 bits) is ignored here too; a configured 0 is honoured.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct InterfaceSettings {
    /// `interface_mode`, else `mode`, with every alias.
    pub mode: InterfaceMode,
    /// From `networkname`/`network_name`, `passphrase`/`pass_phrase` and `ifac_size` in
    /// bits, with the carrier's default code size (`Reticulum.py` 884-902).
    pub ifac: Option<Ifac>,
    /// `bitrate`, at least 5.
    pub bitrate: Option<u64>,
    /// `announce_cap` in percent, rounded to a whole percent of at least 1.
    pub announce_cap: Option<u8>,
    /// `outgoing`.
    pub outgoing: bool,
    /// `ingress_control` and the `ic_*` keys over RNS's defaults; `None` when none is set.
    pub ingress: Option<AnnounceIngressPolicy>,
    /// `announce_rate_target`, `_grace` and `_penalty`; grace and penalty default to 0
    /// when only a target is set (`Reticulum.py` 935-946).
    pub announce_rate: Option<AnnounceRate>,
    /// `gravity`.
    pub gravity: i16,
    /// `recursive_prs`, `egress_control`, `ec_pr_freq`, `ic_pr_burst_freq` and
    /// `ic_pr_burst_freq_new`.
    pub path_requests: PathRequestPolicy,
    /// `announces_from_internal`.
    pub announces_from_internal: bool,
    /// `announces_to_internal`.
    pub announces_to_internal: bool,
    /// `bootstrap_only`. Parsed, not supported: retinue has no interface discovery.
    pub bootstrap_only: bool,
}

/// RNS's mode names (`Reticulum.py` 847-877).
fn mode(value: &str) -> Option<InterfaceMode> {
    Some(match value.to_lowercase().as_str() {
        "full" => InterfaceMode::Full,
        "access_point" | "accesspoint" | "ap" => InterfaceMode::AccessPoint,
        "pointtopoint" | "ptp" => InterfaceMode::PointToPoint,
        "roaming" => InterfaceMode::Roaming,
        "boundary" => InterfaceMode::Boundary,
        "gateway" | "gw" => InterfaceMode::Gateway,
        "internal" => InterfaceMode::Internal,
        _ => return None,
    })
}

impl InterfaceSettings {
    pub(super) fn from_keys(keys: &Keys<'_>, ifac_default: usize) -> Result<Self, RnsConfigError> {
        // `interface_mode` wins over `mode`, and its own value decides gateway and internal.
        let mode = keys
            .str("interface_mode")
            .or_else(|| keys.str("mode"))
            .and_then(mode)
            .unwrap_or_default();

        // The later spelling wins only when it is set (`Reticulum.py` 888-902).
        let set = |key| keys.str(key).filter(|v| !v.is_empty() && *v != "None");
        let name = set("network_name").or_else(|| set("networkname"));
        let pass = set("pass_phrase").or_else(|| set("passphrase"));
        let bits = keys.int::<u32>("ifac_size")?;
        let ifac = Ifac::from_config_bits(name, pass, bits, ifac_default).map_err(|error| {
            RnsConfigError::Ifac {
                interface: keys.name.into(),
                error,
            }
        })?;

        let target = keys.int::<u64>("announce_rate_target")?.filter(|&t| t > 0);
        let grace = keys.int::<u16>("announce_rate_grace")?;
        let penalty = keys.int::<u64>("announce_rate_penalty")?;
        let announce_rate = target.map(|target| AnnounceRate {
            target: Duration::from_secs(target),
            grace: grace.unwrap_or(0),
            penalty: Duration::from_secs(penalty.unwrap_or(0)),
        });

        let defaults = PathRequestPolicy::default();
        let path_requests = PathRequestPolicy {
            recursive: keys.bool("recursive_prs")?.unwrap_or(defaults.recursive),
            egress_control: keys
                .bool("egress_control")?
                .unwrap_or(defaults.egress_control),
            egress_hz: keys.hz("ec_pr_freq")?.unwrap_or(defaults.egress_hz),
            burst_hz_new: keys
                .hz("ic_pr_burst_freq_new")?
                .unwrap_or(defaults.burst_hz_new),
            burst_hz: keys.hz("ic_pr_burst_freq")?.unwrap_or(defaults.burst_hz),
        };

        Ok(Self {
            mode,
            ifac,
            bitrate: keys.int::<u64>("bitrate")?.filter(|&b| b >= 5),
            announce_cap: keys
                .float("announce_cap")?
                .filter(|&c| c > 0.0 && c <= 100.0)
                .map(|c| c.round().max(1.0) as u8),
            outgoing: keys.bool("outgoing")?.unwrap_or(true),
            ingress: ingress(keys)?,
            announce_rate,
            gravity: keys.int("gravity")?.unwrap_or(0),
            path_requests,
            announces_from_internal: keys.bool("announces_from_internal")?.unwrap_or(true),
            announces_to_internal: keys.bool("announces_to_internal")?.unwrap_or(false),
            bootstrap_only: keys.bool("bootstrap_only")?.unwrap_or(false),
        })
    }

    /// The policy a listener's or Auto parent's spawned interfaces inherit.
    pub fn iface_policy(&self) -> IfacePolicy {
        let defaults = IfacePolicy::default();
        IfacePolicy {
            ingress: self.ingress,
            announce_rate: self.announce_rate,
            cap_percent: self.announce_cap.unwrap_or(defaults.cap_percent),
            bitrate_bps: self.bitrate,
            gravity: self.gravity,
            outgoing: self.outgoing,
            announces_from_internal: self.announces_from_internal,
            announces_to_internal: self.announces_to_internal,
            path_requests: self.path_requests,
        }
    }
}

/// `ingress_control` and the `ic_*` overrides (`Reticulum.py` 904-927, 1066-1078).
fn ingress(keys: &Keys<'_>) -> Result<Option<AnnounceIngressPolicy>, RnsConfigError> {
    let mut policy = AnnounceIngressPolicy::default();
    let mut set = false;
    let mut apply = |value: Option<()>| set |= value.is_some();
    apply(keys.bool("ingress_control")?.map(|on| policy.enabled = on));
    apply(
        keys.int("ic_max_held_announces")?
            .map(|n| policy.held_capacity = n),
    );
    apply(
        keys.duration("ic_burst_hold", 1.0)?
            .map(|d| policy.burst_hold = d),
    );
    apply(
        keys.hz("ic_burst_freq_new")?
            .map(|hz| policy.new_interface_hz = hz),
    );
    apply(
        keys.hz("ic_burst_freq")?
            .map(|hz| policy.established_interface_hz = hz),
    );
    apply(
        keys.duration("ic_new_time", 1.0)?
            .map(|d| policy.new_interface_age = d),
    );
    apply(
        keys.duration("ic_burst_penalty", 1.0)?
            .map(|d| policy.burst_penalty = d),
    );
    apply(
        keys.duration("ic_held_release_interval", 1.0)?
            .map(|d| policy.held_release_interval = d),
    );
    Ok(set.then_some(policy))
}

impl Endpoint {
    /// Apply an RNS section's settings to an attached interface through its public setters.
    /// IFAC is the carrier's, set when it attaches. False if the interface is not attached
    /// or a setter refused its value.
    pub fn apply_interface_settings(&self, id: InterfaceId, settings: &InterfaceSettings) -> bool {
        let applied = [
            self.set_interface_mode(id, settings.mode),
            self.set_interface_ingress_policy(id, settings.ingress),
            self.set_interface_announce_rate(id, settings.announce_rate),
            settings
                .announce_cap
                .is_none_or(|cap| self.set_announce_cap(id, cap)),
            self.set_interface_bitrate(id, settings.bitrate),
            self.set_interface_gravity(id, settings.gravity),
            self.set_interface_outgoing(id, settings.outgoing),
            self.set_interface_mode_flags(
                id,
                settings.announces_from_internal,
                settings.announces_to_internal,
            ),
            self.set_interface_path_request_policy(id, settings.path_requests),
        ];
        applied.iter().all(|&ok| ok)
    }
}
