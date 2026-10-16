//! Pump settings and lifecycle status.

use std::time::Duration;

/// Runtime policy for the serial pump. Durations are settings because USB devices and
/// half-duplex radios need different settle and turnaround margins.
#[derive(Clone, Debug)]
pub struct SerialPumpConfig {
    /// Serial line rate used by RNode firmware.
    pub baud_rate: u32,
    /// Time allowed for a board reset triggered by opening its USB serial port.
    pub open_settle: Duration,
    /// How often to repeat the RNode detect/configuration conversation until it is online.
    pub init_retry: Duration,
    /// Receive window left after each packet's calculated airtime.
    pub turnaround: Duration,
    /// Retry delay when the modem reports its half-duplex queue busy.
    pub busy_retry: Duration,
    /// Maximum number of frames waiting for the radio.
    pub tx_queue: usize,
    /// Maximum number of received frames waiting for the protocol consumer.
    pub rx_queue: usize,
    /// A half-read frame is discarded after this long without bytes (`RNodeInterface.py`
    /// 1137-1143).
    pub idle_reset: Duration,
    /// A flow-control lock the device never releases is lifted after this.
    pub flow_unlock: Duration,
    /// Delay before a supervised link reopens a failed port (`RNodeInterface.py` 1175-1187).
    pub reconnect: Duration,
    /// Send the detect probe whenever nothing was written for this long, so a dead peer
    /// shows up as a write error (TCP: `RNodeInterface.py` 1151-1154, 1457-1458).
    pub keepalive: Option<Duration>,
    /// Give up on the port, for a supervisor to reopen, if the device has not answered
    /// DETECT this long after the probe (TCP: `RNodeInterface.py` 437-441).
    pub detect_timeout: Option<Duration>,
}

impl SerialPumpConfig {
    /// RNS's profile for an RNode over TCP: 2 s settle, a 1.5 s partial-frame timeout, a
    /// 5 s detect wait and a detect keepalive after 3.5 s without writes
    /// (`RNodeInterface.py` 402-441, 1151-1154, 1452-1458).
    pub fn tcp() -> Self {
        Self {
            open_settle: Duration::from_secs(2),
            idle_reset: Duration::from_millis(1_500),
            keepalive: Some(Duration::from_millis(3_500)),
            detect_timeout: Some(Duration::from_secs(5)),
            ..Self::default()
        }
    }
}

impl Default for SerialPumpConfig {
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            open_settle: Duration::from_secs(3),
            init_retry: Duration::from_secs(6),
            turnaround: Duration::from_millis(180),
            busy_retry: Duration::from_millis(50),
            tx_queue: 32,
            rx_queue: 32,
            idle_reset: Duration::from_millis(100),
            flow_unlock: Duration::from_secs(5),
            reconnect: Duration::from_secs(5),
            keepalive: None,
            detect_timeout: None,
        }
    }
}

/// RNS's port for an RNode reached over TCP (`RNodeInterface.py` 1453).
pub const RNODE_TCP_PORT: u16 = 7633;

/// Where an RNode is reached: RNS's `port` key (`RNodeInterface.py` 170-193).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RNodePort {
    /// A serial device path.
    Serial(String),
    /// `tcp://host`, on [`RNODE_TCP_PORT`] unless a test overrides it.
    Tcp { host: String, port: u16 },
}

impl RNodePort {
    /// Parse RNS's `port` value: `tcp://host` (scheme case-insensitive) or a device path.
    /// `None` for `tcp://` without a host, which RNS would leave unresolved, and for
    /// `ble://`, which is not supported.
    pub fn parse(value: &str) -> Option<Self> {
        let scheme = |name: &str| {
            value
                .get(..name.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(name))
        };
        if scheme("ble://") {
            return None;
        }
        if scheme("tcp://") {
            let host = &value["tcp://".len()..];
            return (!host.is_empty()).then(|| Self::Tcp {
                host: host.to_string(),
                port: RNODE_TCP_PORT,
            });
        }
        Some(Self::Serial(value.to_string()))
    }
}

/// Observable lifecycle of the pump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PumpStatus {
    Settling,
    Initializing,
    Online { firmware: Option<(u8, u8)> },
    Fault(String),
    Stopped,
}
