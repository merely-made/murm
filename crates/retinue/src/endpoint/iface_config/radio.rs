//! Serial, KISS, AX.25 KISS and RNode sections (`SerialInterface.py` 76-103;
//! `KISSInterface.py` 86-119; `AX25KISSInterface.py` 93-116; `RNodeInterface.py` 150-195).

#[cfg(feature = "tulle-radio")]
use alloc::string::String;
use alloc::string::ToString;

use std::time::Duration;

use super::carriers::{Carrier, IdBeacon};
use super::{Keys, RnsConfigError};

#[cfg(feature = "serial")]
pub(super) fn serial(kind: &str, keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    use crate::iface::kiss::TncConfig;
    use crate::iface::serial::{Parity, SerialConfig};

    let path = keys.str("port").ok_or_else(|| keys.missing("port"))?;
    let mut port = SerialConfig::new(path);
    if let Some(speed) = keys.int("speed")? {
        port.speed = speed;
    }
    if let Some(bits) = keys.int("databits")? {
        port.databits = bits;
    }
    if let Some(bits) = keys.int("stopbits")? {
        port.stopbits = bits;
    }
    port.parity = keys
        .parsed("parity", |v| match v.to_lowercase().as_str() {
            "n" | "none" => Some(Parity::None),
            "e" | "even" => Some(Parity::Even),
            "o" | "odd" => Some(Parity::Odd),
            _ => None,
        })?
        .unwrap_or_default();
    if kind == "SerialInterface" {
        return Ok(Carrier::Serial(port));
    }

    let mut tnc = TncConfig::default();
    if let Some(ms) = keys.int("preamble")? {
        tnc.preamble_ms = ms;
    }
    if let Some(ms) = keys.int("txtail")? {
        tnc.txtail_ms = ms;
    }
    if let Some(p) = keys.int("persistence")? {
        tnc.persistence = p;
    }
    if let Some(ms) = keys.int("slottime")? {
        tnc.slottime_ms = ms;
    }
    tnc.flow_control = keys.bool("flow_control")?.unwrap_or(false);
    if kind == "AX25KISSInterface" {
        // RNS refuses a callsign outside 3-6 characters and its default SSID of -1
        // (`AX25KISSInterface.py` 103-104, 136-140).
        let callsign = keys.str("callsign").unwrap_or("");
        let ssid = keys.int::<i64>("ssid")?.unwrap_or(-1);
        let ssid = u8::try_from(ssid).map_err(|_| keys.bad("ssid", &ssid.to_string()))?;
        let header = crate::iface::ax25::Ax25Header::new(callsign, ssid).map_err(|e| match e {
            crate::iface::ax25::Ax25Error::Callsign => keys.bad("callsign", callsign),
            crate::iface::ax25::Ax25Error::Ssid => keys.bad("ssid", &ssid.to_string()),
        })?;
        return Ok(Carrier::Ax25Kiss { port, tnc, header });
    }
    // KISS beacons whenever an interval is set, an unset callsign being empty.
    let beacon = keys.int::<u64>("id_interval")?.map(|secs| IdBeacon {
        callsign: keys.str("id_callsign").unwrap_or("").to_string(),
        interval: Duration::from_secs(secs),
    });
    Ok(Carrier::Kiss { port, tnc, beacon })
}

#[cfg(feature = "tulle-radio")]
pub(super) fn rnode(keys: &Keys<'_>) -> Result<Carrier, RnsConfigError> {
    use tulle::lora::{CodingRate, LoRaParams};
    use tulle::rnode::RNodeConfig;
    use tulle::serial::RNodePort;

    let value = keys.str("port").ok_or_else(|| keys.missing("port"))?;
    let port = RNodePort::parse(value).ok_or_else(|| keys.bad("port", value))?;
    let required = |key: &str| keys.int::<u32>(key)?.ok_or_else(|| keys.missing(key));
    let coding_rate = match required("codingrate")? {
        5 => CodingRate::Cr45,
        6 => CodingRate::Cr46,
        7 => CodingRate::Cr47,
        8 => CodingRate::Cr48,
        cr => return Err(keys.bad("codingrate", &cr.to_string())),
    };
    let params = LoRaParams {
        spreading_factor: u8::try_from(required("spreadingfactor")?)
            .map_err(|_| keys.bad("spreadingfactor", keys.str("spreadingfactor").unwrap_or("")))?,
        bandwidth_hz: required("bandwidth")?,
        coding_rate,
        frequency_hz: required("frequency")?,
        tx_power_dbm: keys.int("txpower")?.unwrap_or(0),
        preamble_syms: 8,
        explicit_header: true,
        crc: true,
    };
    let mut config = RNodeConfig::new(params);
    // Airtime limits in percent, sent as hundredths (`RNodeInterface.py` 619-641).
    let alock = |key| {
        keys.parsed(key, |v: &str| {
            let pct = v.trim().parse::<f64>().ok()?;
            (0.0..=100.0).contains(&pct).then_some((pct * 100.0) as u16)
        })
    };
    config.st_alock = alock("airtime_limit_short")?;
    config.lt_alock = alock("airtime_limit_long")?;
    config.flow_control = keys.bool("flow_control")?.unwrap_or(false);
    config
        .validate()
        .map_err(|e| keys.bad("radio parameters", &e.to_string()))?;
    let beacon = match (keys.int::<u64>("id_interval")?, keys.str("id_callsign")) {
        (Some(secs), Some(callsign)) => {
            if callsign.len() > crate::iface::beacon::CALLSIGN_MAX_LEN {
                return Err(keys.bad("id_callsign", callsign));
            }
            Some(IdBeacon {
                callsign: String::from(callsign),
                interval: Duration::from_secs(secs),
            })
        }
        _ => None,
    };
    Ok(Carrier::RNode {
        port,
        config,
        beacon,
    })
}
