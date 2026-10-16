//! Native companion acceptance over real controls and actual journal facts.
//! This lane creates no identities/messages and leaves hardware activation to
//! the production Connect callback. Its script must supply identified ports.
use crate::{
    DesktopState,
    appearance_host::Context,
    views::{Child, Logic},
};
use genet_probe::ProbeSnapshot;
use layout_dom_api::LayoutDom;
use signalman::comms_adapter::comms::{DeliveryStatus, Direction, ProtocolKind};
use std::{cell::Cell, path::PathBuf};

pub struct Product {
    sheet: String,
    tag: String,
    protocol: ProtocolKind,
    peer: String,
    peer_receipt: Option<PathBuf>,
    condition: Option<Condition>,
    connected_seen: Cell<bool>,
    history: Option<serde_json::Value>,
    reconnect: Option<Reconnect>,
    previous_exchange: Option<(String, Option<PathBuf>)>,
    channel_only: bool,
}
struct Reconnect {
    tag: String,
    peer_receipt: PathBuf,
    ready: PathBuf,
}
enum Condition {
    Connected,
    Stopped,
    Acknowledged,
    Received,
    PeerAcknowledged,
}

fn peer_confirmed(
    receipt: &serde_json::Value,
    protocol: ProtocolKind,
    peer: &str,
    tag: &str,
) -> bool {
    let protocol = match protocol {
        ProtocolKind::MeshCore => "meshcore",
        ProtocolKind::Meshtastic => "meshtastic",
        _ => return false,
    };
    receipt["result"] == "passed"
        && receipt["mode"] == "peer"
        && receipt["protocol"] == protocol
        && receipt["ui_peer"]["matched_protocol_ack"] == true
        && receipt["ui_peer"]["received"]["message"]["body"]["PlainText"] == tag
        && receipt["ui_peer"]["sent"]["message"]["body"]["PlainText"] == format!("{tag}-return")
        && receipt["ui_peer"]["sent"]["message"]["author"]["address"] == peer
        && receipt["ui_peer"]["sent"]["delivery"] == "ProtocolAcknowledged"
}
impl Product {
    fn observed_protocol_acks(&self, state: &DesktopState) -> usize {
        state
            .radio_owner(self.protocol)
            .and_then(|owner| owner.records().ok())
            .unwrap_or_default()
            .iter()
            .filter(|record| record.delivery == DeliveryStatus::ProtocolAcknowledged)
            .count()
    }
    fn peer_acknowledged(&self) -> bool {
        peer_acknowledged(
            self.peer_receipt.as_ref(),
            self.protocol,
            &self.peer,
            &self.tag,
            self.channel_only,
        )
    }
}
fn peer_acknowledged(
    path: Option<&PathBuf>,
    protocol: ProtocolKind,
    peer: &str,
    tag: &str,
    channel_only: bool,
) -> bool {
    path.and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .is_some_and(|receipt| {
            if channel_only {
                channel_peer_confirmed(&receipt, peer, tag)
            } else {
                peer_confirmed(&receipt, protocol, peer, tag)
            }
        })
}
fn channel_peer_confirmed(receipt: &serde_json::Value, peer: &str, tag: &str) -> bool {
    receipt["result"] == "passed"
        && receipt["mode"] == "channel-peer"
        && receipt["protocol"] == "meshtastic"
        && receipt["ui_peer"]["broadcast_only"] == true
        && receipt["ui_peer"]["matched_protocol_ack"] == false
        && receipt["ui_peer"]["stock_device_accepted"] == true
        && receipt["ui_peer"]["received"]["message"]["body"]["PlainText"] == tag
        && receipt["ui_peer"]["sent"]["message"]["body"]["PlainText"] == format!("{tag}-return")
        && receipt["ui_peer"]["sent"]["message"]["author"]["address"] == peer
        && receipt["ui_peer"]["sent"]["delivery"] == "HandedToRadio"
}
fn facts(
    state: &DesktopState,
    tag: &str,
    protocol: ProtocolKind,
    peer: &str,
    channel_only: bool,
) -> (bool, usize, usize) {
    let connected =
        state.companion_stops.iter().any(|(kind, stop)| {
            *kind == protocol && !stop.load(std::sync::atomic::Ordering::Relaxed)
        }) && state
            .companion_comms
            .iter()
            .any(|(kind, _)| *kind == protocol);
    let owner = state.radio_owner(protocol);
    let conversation = owner.and_then(|owner| {
        owner
            .conversation(protocol, if channel_only { "ffffffff" } else { peer })
            .ok()
    });
    let records = owner
        .and_then(|owner| owner.records().ok())
        .unwrap_or_default();
    let ack = records
        .iter()
        .filter(|r| {
            r.message.direction == Direction::Outgoing
                && conversation.as_ref() == Some(&r.conversation)
                && r.message.body.text() == tag
                && r.delivery
                    == (if channel_only {
                        DeliveryStatus::HandedToRadio
                    } else {
                        DeliveryStatus::ProtocolAcknowledged
                    })
        })
        .count();
    let rx = records
        .iter()
        .filter(|r| {
            r.message.direction == Direction::Incoming
                && r.message.author.address == peer
                && conversation.as_ref() == Some(&r.conversation)
                && r.message.body.text() == format!("{tag}-return")
                && r.delivery == DeliveryStatus::ReceivedByRadio
        })
        .count();
    (connected, ack, rx)
}
pub fn from_env() -> Option<mesquite::Lane<Product>> {
    let config = mesquite::LaneConfig::from_env("SIGNALMAN_COMPANION")?;
    let tag = std::env::var("SIGNALMAN_COMPANION_TAG")
        .expect("native companion scenario requires its explicit bench tag");
    assert!(
        tag.starts_with("signalman-ui-")
            && tag.len() <= 100
            && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid native companion bench tag"
    );
    let protocol = match std::env::var("SIGNALMAN_COMPANION_PROTOCOL").as_deref() {
        Ok("meshtastic") => ProtocolKind::Meshtastic,
        Ok("meshcore") => ProtocolKind::MeshCore,
        _ => panic!("native companion scenario requires meshtastic or meshcore protocol"),
    };
    let channel_only = std::env::var("SIGNALMAN_COMPANION_CHANNEL_ONLY").as_deref() == Ok("1");
    assert!(
        !channel_only || protocol == ProtocolKind::Meshtastic,
        "channel-only fixture requires Meshtastic"
    );
    let peer = std::env::var("SIGNALMAN_COMPANION_PEER")
        .expect("native companion scenario requires its identified peer");
    let length = if protocol == ProtocolKind::Meshtastic {
        8
    } else {
        64
    };
    assert!(
        peer.len() == length
            && peer
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "invalid native companion peer address"
    );
    let reconnect =
        std::env::var_os("SIGNALMAN_COMPANION_RECONNECT_PEER_RECEIPT").map(|path| Reconnect {
            tag: format!("{tag}-reconnect"),
            peer_receipt: PathBuf::from(path),
            ready: PathBuf::from(
                std::env::var_os("SIGNALMAN_COMPANION_RECONNECT_READY")
                    .expect("reconnect exchange requires its explicit ready marker"),
            ),
        });
    Some(
        mesquite::Lane::from_config(
            config,
            Product {
                sheet: crate::theme::sheet(),
                tag,
                protocol,
                peer,
                peer_receipt: std::env::var_os("SIGNALMAN_COMPANION_PEER_RECEIPT")
                    .map(PathBuf::from),
                condition: None,
                connected_seen: Cell::new(false),
                history: None,
                reconnect,
                previous_exchange: None,
                channel_only,
            },
            cambium_genet_winit_host::read_file,
        )
        .expect("load native companion scenario")
        .with_frame_limit(Some(7200)),
    )
}
pub fn drive(lane: &mut mesquite::Lane<Product>, ctx: &mut Context<'_>) {
    lane.product_mut().sheet = ctx.runner.state().appearance.stylesheet();
    lane.after_frame(ctx);
}
impl mesquite::Product for Product {
    type State = DesktopState;
    type Logic = Logic;
    type View = Child;
    const KIND: &'static str = "signalman-stock-companion";
    const SURFACE: &'static str = "app";
    const LOG_PREFIX: &'static str = "signalman-stock-companion";
    fn sheet(&self) -> &str {
        &self.sheet
    }
    fn snapshot(&self, ctx: &Context<'_>, captures: usize, _: f32) -> ProbeSnapshot {
        let (connected, ack, rx) = facts(
            ctx.runner.state(),
            &self.tag,
            self.protocol,
            &self.peer,
            self.channel_only,
        );
        if connected {
            self.connected_seen.set(true);
        }
        ProbeSnapshot::default()
            .with_field("connected", connected.to_string())
            .with_field(
                "protocol_ack",
                if self.channel_only {
                    self.observed_protocol_acks(ctx.runner.state()).to_string()
                } else {
                    ack.to_string()
                },
            )
            .with_field(
                "physical_tx",
                if self.channel_only {
                    ack.to_string()
                } else {
                    "0".into()
                },
            )
            .with_field("incoming", rx.to_string())
            .with_field(
                "peer_protocol_ack",
                (!self.channel_only && self.peer_acknowledged()).to_string(),
            )
            .with_field(
                "peer_channel_completed",
                (self.channel_only && self.peer_acknowledged()).to_string(),
            )
            .with_field("captures", captures.to_string())
            .with_field("port", ctx.runner.state().companion_port.text())
            .with_field("realm", ctx.runner.state().companion_realm.text())
            .with_field("journal", ctx.runner.state().companion_journal.text())
            .with_field("channel", ctx.runner.state().companion_channel.text())
            .with_field(
                "protocol_selected",
                (ctx.runner.state().message_protocol == self.protocol).to_string(),
            )
    }
    fn app_step(
        &mut self,
        ctx: &mut Context<'_>,
        _: mesquite::Checkpoints<'_>,
        line: &str,
    ) -> Result<(), String> {
        if line == "begin companion reconnect exchange" {
            if self.previous_exchange.is_some() {
                return Err("only one post-reconnect exchange is configured".into());
            }
            let (connected, ack, rx) = facts(
                ctx.runner.state(),
                &self.tag,
                self.protocol,
                &self.peer,
                self.channel_only,
            );
            let records = ctx
                .runner
                .state()
                .radio_owner(self.protocol)
                .ok_or("reconnect has no commissioned owner")?
                .records()
                .map_err(|e| e.to_string())?;
            let history = serde_json::to_value(records).map_err(|e| e.to_string())?;
            if !connected
                || ack != 1
                || rx != 1
                || !self.peer_acknowledged()
                || self.history.as_ref() != Some(&history)
            {
                return Err(
                    "reconnect exchange requires the first round trip and exact reopened history"
                        .into(),
                );
            }
            let next = self
                .reconnect
                .take()
                .ok_or("reconnect exchange is not configured")?;
            self.previous_exchange = Some((
                std::mem::replace(&mut self.tag, next.tag),
                self.peer_receipt.replace(next.peer_receipt),
            ));
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(next.ready)
                .map_err(|e| format!("reconnect readiness marker: {e}"))?;
            return Ok(());
        }
        if matches!(
            line,
            "remember companion history" | "assert companion history"
        ) {
            let records = ctx
                .runner
                .state()
                .radio_owner(self.protocol)
                .ok_or("history has no commissioned owner")?
                .records()
                .map_err(|e| e.to_string())?;
            let value = serde_json::to_value(records).map_err(|e| e.to_string())?;
            if line == "remember companion history" {
                let (_, ack, rx) = facts(
                    ctx.runner.state(),
                    &self.tag,
                    self.protocol,
                    &self.peer,
                    self.channel_only,
                );
                if ack != 1 || rx != 1 {
                    return Err("history checkpoint requires the matched round trip".into());
                }
                self.history = Some(value);
            } else if self.history.as_ref() != Some(&value) {
                return Err("identity-bound message history changed across reconnect".into());
            }
            return Ok(());
        }
        if let Some(target) = line.strip_prefix("key tab-until data-text-field=") {
            for _ in 0..200 {
                let matched = ctx.runner.focus().is_some_and(|node| {
                    let dom = ctx.runner.dom();
                    let dom = dom.borrow();
                    let mut cursor = Some(node);
                    while let Some(current) = cursor {
                        if dom.attributes(current).any(|a| {
                            a.name.local.as_ref() == "data-text-field" && a.value == target
                        }) {
                            return true;
                        }
                        cursor = dom.parent(current);
                    }
                    false
                });
                if matched {
                    return Ok(());
                }
                ctx.runner
                    .dispatch_key(cambium::KeyEvent::new(cambium::Key::Named(
                        cambium::NamedKey::Tab,
                    )));
            }
            return Err(format!("Tab never reached text field {target}"));
        }
        if matches!(
            line,
            "await companion peer acknowledged" | "await companion peer completed"
        ) && self.peer_receipt.is_none()
        {
            return Err("native peer acknowledgement requires its explicit receipt path".into());
        }
        self.condition = Some(match line {
            "await companion connected" => Condition::Connected,
            "await companion stopped" => Condition::Stopped,
            "await companion acknowledged" if !self.channel_only => Condition::Acknowledged,
            "await companion transmitted" if self.channel_only => Condition::Acknowledged,
            "await companion received" => Condition::Received,
            "await companion peer acknowledged" if !self.channel_only => {
                Condition::PeerAcknowledged
            }
            "await companion peer completed" if self.channel_only => Condition::PeerAcknowledged,
            _ => return Err(format!("unknown companion scenario verb: {line}")),
        });
        Ok(())
    }
    fn busy_mut(&mut self, ctx: &mut Context<'_>, capture_pending: bool) -> Option<bool> {
        let (connected, ack, rx) = facts(
            ctx.runner.state(),
            &self.tag,
            self.protocol,
            &self.peer,
            self.channel_only,
        );
        if connected {
            self.connected_seen.set(true);
        }
        let waiting = match self.condition {
            Some(Condition::Connected) => !connected,
            Some(Condition::Stopped) => {
                connected
                    || (matches!(
                        self.protocol,
                        ProtocolKind::MeshCore | ProtocolKind::Meshtastic
                    ) && ctx.runner.state().resident_connecting)
                    || ctx
                        .runner
                        .state()
                        .companion_settings
                        .iter()
                        .any(|s| s.protocol.kind() == self.protocol)
            }
            Some(Condition::Acknowledged) => ack == 0,
            Some(Condition::Received) => rx == 0,
            Some(Condition::PeerAcknowledged) => !self.peer_acknowledged(),
            None => false,
        };
        if !waiting {
            self.condition = None;
        }
        Some(capture_pending || waiting)
    }
    fn complete(&mut self, ctx: &mut Context<'_>, _: &genet_probe::Outcome) -> Result<(), String> {
        let (connected, ack, rx) = facts(
            ctx.runner.state(),
            &self.tag,
            self.protocol,
            &self.peer,
            self.channel_only,
        );
        let previous_complete = self.previous_exchange.as_ref().is_none_or(|(tag, path)| {
            let (_, ack, rx) = facts(
                ctx.runner.state(),
                tag,
                self.protocol,
                &self.peer,
                self.channel_only,
            );
            ack == 1
                && rx == 1
                && peer_acknowledged(
                    path.as_ref(),
                    self.protocol,
                    &self.peer,
                    tag,
                    self.channel_only,
                )
        });
        if !self.connected_seen.get()
            || connected
            || (matches!(
                self.protocol,
                ProtocolKind::MeshCore | ProtocolKind::Meshtastic
            ) && ctx.runner.state().resident_connecting)
            || ack != 1
            || rx != 1
            || !previous_complete
            || self.reconnect.is_some()
            || (self.channel_only && self.observed_protocol_acks(ctx.runner.state()) != 0)
            || (self.peer_receipt.is_some() && !self.peer_acknowledged())
            || ctx.runner.state().message_protocol != self.protocol
        {
            return Err(format!(
                "native companion acceptance incomplete: connected={connected}, ack={ack}, receive={rx}"
            ));
        }
        Ok(())
    }
    fn receipt_checks(&self, captures: &[mesquite::CaptureRecord]) -> Vec<String> {
        let required = if self.previous_exchange.is_some() {
            7
        } else {
            6
        };
        if captures.len() < required {
            vec![format!(
                "{required} native captures required, including each completed round trip"
            )]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_gate_uses_device_acceptance_and_refuses_protocol_ack_claims_or_stale_tags() {
        let receipt = serde_json::json!({"mode":"channel-peer","protocol":"meshtastic","result":"passed",
            "ui_peer":{"broadcast_only":true,"matched_protocol_ack":false,"stock_device_accepted":true,
            "received":{"message":{"body":{"PlainText":"signalman-ui-channel"}}},
            "sent":{"message":{"body":{"PlainText":"signalman-ui-channel-return"},"author":{"address":"peer"}},"delivery":"HandedToRadio"}}});
        assert!(channel_peer_confirmed(
            &receipt,
            "peer",
            "signalman-ui-channel"
        ));
        assert!(!channel_peer_confirmed(
            &receipt,
            "peer",
            "signalman-ui-channel-reconnect"
        ));
        assert!(!peer_confirmed(
            &receipt,
            ProtocolKind::Meshtastic,
            "peer",
            "signalman-ui-channel"
        ));
        let mut claimed = receipt;
        claimed["ui_peer"]["matched_protocol_ack"] = true.into();
        claimed["ui_peer"]["sent"]["delivery"] = "ProtocolAcknowledged".into();
        assert!(!channel_peer_confirmed(
            &claimed,
            "peer",
            "signalman-ui-channel"
        ));
    }

    #[test]
    fn peer_gate_requires_the_matching_return_and_observed_ack() {
        let receipt = serde_json::json!({"mode":"peer","protocol":"meshcore","result":"passed",
            "ui_peer":{"matched_protocol_ack":true,
            "received":{"message":{"body":{"PlainText":"signalman-ui-check"}}},
            "sent":{"message":{"body":{"PlainText":"signalman-ui-check-return"},"author":{"address":"peer"}},"delivery":"ProtocolAcknowledged"}}});
        assert!(peer_confirmed(
            &receipt,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-check"
        ));
        assert!(!peer_confirmed(
            &receipt,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-stale"
        ));
        assert!(!peer_confirmed(
            &receipt,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-check-reconnect"
        ));
        assert!(!peer_confirmed(
            &receipt,
            ProtocolKind::MeshCore,
            "other",
            "signalman-ui-check"
        ));
        let mut handed = receipt.clone();
        handed["ui_peer"]["sent"]["delivery"] = "HandedToRadio".into();
        assert!(!peer_confirmed(
            &handed,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-check"
        ));
        let mut incomplete = receipt;
        incomplete["ui_peer"]["matched_protocol_ack"] = false.into();
        assert!(!peer_confirmed(
            &incomplete,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-check"
        ));
        assert!(!peer_confirmed(
            &serde_json::Value::Null,
            ProtocolKind::MeshCore,
            "peer",
            "signalman-ui-check"
        ));
    }
}
