//! RNS config text mapped onto retinue's interface settings (`Reticulum.py` 478-500,
//! 843-1160).
//!
//! [`RnsConfig::parse`] reads the `[reticulum]` shared-instance keys and each `[[name]]`
//! section of `[interfaces]` into an [`RnsInterface`]: its [`InterfaceSettings`] and the
//! [`Carrier`] with retinue's own config types. Two upstream parsing defects are not
//! ported: `interface_mode = gateway` or `internal` without a `mode` key raises KeyError
//! (859-862), and a configured 0 reverting to the default (1271-1314).

mod carriers;
#[cfg(any(feature = "serial", feature = "tulle-radio"))]
mod radio;
mod settings;
mod syntax;
#[cfg(test)]
mod tests;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use std::time::Duration;

use crate::ifac::ConfigError;

use super::local::{SharedInstance, SharedInstanceKind};

pub use carriers::{Carrier, IdBeacon, RnsAttached, UdpCarrier};
pub use settings::InterfaceSettings;

/// A parsed RNS config: the shared instance and each interface section, in file order.
#[derive(Clone, Debug)]
pub struct RnsConfig {
    /// `[reticulum]` `instance_name`, `shared_instance_port` and `shared_instance_type`.
    pub shared_instance: SharedInstance,
    /// Each `[[name]]` section, or why it cannot be mapped.
    pub interfaces: Vec<Result<RnsInterface, RnsConfigError>>,
}

/// One `[[name]]` section of `[interfaces]`.
#[derive(Clone, Debug)]
pub struct RnsInterface {
    pub name: String,
    /// `enabled` or `interface_enabled`; RNS attaches only enabled interfaces.
    pub enabled: bool,
    pub settings: InterfaceSettings,
    pub carrier: Carrier,
}

/// Why config text, or one of its sections, cannot be mapped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RnsConfigError {
    /// A line that is not a section header, `key = value`, comment or blank, or a
    /// duplicate section or key.
    Syntax { line: usize },
    /// A value RNS would refuse or fail on.
    Value {
        interface: String,
        key: String,
        value: String,
    },
    /// A key the carrier cannot start without.
    Missing { interface: String, key: String },
    /// An interface type retinue has no carrier for: I2P, Weave, RNodeMulti, or one gated
    /// behind a feature this build lacks.
    UnsupportedType { interface: String, kind: String },
    /// IFAC credentials that cannot be derived.
    Ifac {
        interface: String,
        error: ConfigError,
    },
}

impl fmt::Display for RnsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line } => write!(f, "config syntax error on line {line}"),
            Self::Value {
                interface,
                key,
                value,
            } => write!(f, "{interface}: invalid {key} = {value}"),
            Self::Missing { interface, key } => write!(f, "{interface}: {key} is required"),
            Self::UnsupportedType { interface, kind } => {
                write!(f, "{interface}: interface type {kind} is not supported")
            }
            Self::Ifac { interface, error } => write!(f, "{interface}: IFAC: {error}"),
        }
    }
}

impl std::error::Error for RnsConfigError {}

impl RnsConfig {
    /// Parse RNS config text. Only syntax fails the whole config; a section that cannot be
    /// mapped is an error in its place.
    pub fn parse(text: &str) -> Result<Self, RnsConfigError> {
        let doc = syntax::parse(text)?;
        let reticulum = Keys {
            name: "reticulum",
            section: &doc.reticulum,
        };
        let mut shared_instance = SharedInstance::default();
        if let Some(name) = reticulum.str("instance_name") {
            shared_instance.instance_name = name.to_string();
        }
        if let Some(port) = reticulum.int("shared_instance_port")? {
            shared_instance.port = port;
        }
        match reticulum.str("shared_instance_type").map(str::to_lowercase) {
            Some(kind) if kind == "tcp" => shared_instance.kind = SharedInstanceKind::Tcp,
            Some(kind) if kind == "unix" => shared_instance.kind = SharedInstanceKind::Unix,
            _ => {}
        }
        let interfaces = doc
            .interfaces
            .iter()
            .map(|section| {
                RnsInterface::from_keys(&Keys {
                    name: &section.name,
                    section,
                })
            })
            .collect();
        Ok(Self {
            shared_instance,
            interfaces,
        })
    }
}

impl RnsInterface {
    fn from_keys(keys: &Keys<'_>) -> Result<Self, RnsConfigError> {
        let kind = keys.str("type").ok_or_else(|| keys.missing("type"))?;
        let enabled = keys.bool("interface_enabled")?.unwrap_or(false)
            || keys.bool("enabled")?.unwrap_or(false);
        let (carrier, ifac_default) = carriers::carrier(kind, keys)?;
        let settings = InterfaceSettings::from_keys(keys, ifac_default)?;
        let carrier = carrier.with_settings(&settings);
        Ok(Self {
            name: keys.name.to_string(),
            enabled,
            settings,
            carrier,
        })
    }
}

/// Typed reads of one section's values, as ConfigObj's `as_*` accessors.
pub(super) struct Keys<'a> {
    name: &'a str,
    section: &'a syntax::Section,
}

impl<'a> Keys<'a> {
    fn str(&self, key: &str) -> Option<&'a str> {
        self.section
            .keys
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn bad(&self, key: &str, value: &str) -> RnsConfigError {
        RnsConfigError::Value {
            interface: self.name.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn missing(&self, key: &str) -> RnsConfigError {
        RnsConfigError::Missing {
            interface: self.name.to_string(),
            key: key.to_string(),
        }
    }

    fn parsed<T>(
        &self,
        key: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>, RnsConfigError> {
        self.str(key)
            .map(|v| parse(v).ok_or_else(|| self.bad(key, v)))
            .transpose()
    }

    fn bool(&self, key: &str) -> Result<Option<bool>, RnsConfigError> {
        self.parsed(key, syntax::boolean)
    }

    fn int<T: TryFrom<i64>>(&self, key: &str) -> Result<Option<T>, RnsConfigError> {
        self.parsed(key, |v| v.trim().parse::<i64>().ok()?.try_into().ok())
    }

    fn float(&self, key: &str) -> Result<Option<f64>, RnsConfigError> {
        self.parsed(key, |v| {
            v.trim().parse::<f64>().ok().filter(|x| x.is_finite())
        })
    }

    /// Non-negative seconds, scaled by `unit` (60 for minutes).
    fn duration(&self, key: &str, unit: f64) -> Result<Option<Duration>, RnsConfigError> {
        self.parsed(key, |v| {
            let secs = v.trim().parse::<f64>().ok()? * unit;
            Duration::try_from_secs_f64(secs).ok()
        })
    }

    /// A non-negative rate, rounded to whole hertz.
    fn hz(&self, key: &str) -> Result<Option<u64>, RnsConfigError> {
        self.parsed(key, |v| {
            let hz = v.trim().parse::<f64>().ok()?;
            (hz.is_finite() && hz >= 0.0).then(|| hz.round() as u64)
        })
    }
}
