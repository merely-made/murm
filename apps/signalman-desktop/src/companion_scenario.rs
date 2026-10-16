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
use std::cell::Cell;

pub struct Product {
    sheet: String,
    tag: String,
    protocol: ProtocolKind,
    peer: String,
    condition: Option<Condition>,
    connected_seen: Cell<bool>,
    history: Option<serde_json::Value>,
}
enum Condition {
    Connected,
    Stopped,
    Acknowledged,
    Received,
}
fn facts(
    state: &DesktopState,
    tag: &str,
    protocol: ProtocolKind,
    peer: &str,
) -> (bool, usize, usize) {
    let connected =
        state.companion_stops.iter().any(|(kind, stop)| {
            *kind == protocol && !stop.load(std::sync::atomic::Ordering::Relaxed)
        }) && state
            .companion_comms
            .iter()
            .any(|(kind, _)| *kind == protocol);
    let owner = state.radio_owner(protocol);
    let conversation = owner.and_then(|owner| owner.conversation(protocol, peer).ok());
    let records = owner
        .and_then(|owner| owner.records().ok())
        .unwrap_or_default();
    let ack = records
        .iter()
        .filter(|r| {
            r.message.direction == Direction::Outgoing
                && conversation.as_ref() == Some(&r.conversation)
                && r.message.body.text() == tag
                && r.delivery == DeliveryStatus::ProtocolAcknowledged
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
    Some(
        mesquite::Lane::from_config(
            config,
            Product {
                sheet: crate::theme::sheet(),
                tag,
                protocol,
                peer,
                condition: None,
                connected_seen: Cell::new(false),
                history: None,
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
        let (connected, ack, rx) = facts(ctx.runner.state(), &self.tag, self.protocol, &self.peer);
        if connected {
            self.connected_seen.set(true);
        }
        ProbeSnapshot::default()
            .with_field("connected", connected.to_string())
            .with_field("protocol_ack", ack.to_string())
            .with_field("incoming", rx.to_string())
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
                let (_, ack, rx) = facts(ctx.runner.state(), &self.tag, self.protocol, &self.peer);
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
        self.condition = Some(match line {
            "await companion connected" => Condition::Connected,
            "await companion stopped" => Condition::Stopped,
            "await companion acknowledged" => Condition::Acknowledged,
            "await companion received" => Condition::Received,
            _ => return Err(format!("unknown companion scenario verb: {line}")),
        });
        Ok(())
    }
    fn busy_mut(&mut self, ctx: &mut Context<'_>, capture_pending: bool) -> Option<bool> {
        let (connected, ack, rx) = facts(ctx.runner.state(), &self.tag, self.protocol, &self.peer);
        if connected {
            self.connected_seen.set(true);
        }
        let waiting = match self.condition {
            Some(Condition::Connected) => !connected,
            Some(Condition::Stopped) => {
                connected
                    || (self.protocol == ProtocolKind::MeshCore
                        && ctx.runner.state().resident_connecting)
                    || ctx
                        .runner
                        .state()
                        .companion_settings
                        .iter()
                        .any(|s| s.protocol.kind() == self.protocol)
            }
            Some(Condition::Acknowledged) => ack == 0,
            Some(Condition::Received) => rx == 0,
            None => false,
        };
        if !waiting {
            self.condition = None;
        }
        Some(capture_pending || waiting)
    }
    fn complete(&mut self, ctx: &mut Context<'_>, _: &genet_probe::Outcome) -> Result<(), String> {
        let (connected, ack, rx) = facts(ctx.runner.state(), &self.tag, self.protocol, &self.peer);
        if !self.connected_seen.get()
            || connected
            || (self.protocol == ProtocolKind::MeshCore && ctx.runner.state().resident_connecting)
            || ack != 1
            || rx != 1
            || ctx.runner.state().message_protocol != self.protocol
        {
            return Err(format!(
                "native companion acceptance incomplete: connected={connected}, ack={ack}, receive={rx}"
            ));
        }
        Ok(())
    }
    fn receipt_checks(&self, captures: &[mesquite::CaptureRecord]) -> Vec<String> {
        if captures.len() < 6 {
            vec!["six native form/connected/roundtrip/disconnected/reconnected/stopped captures required".into()]
        } else {
            vec![]
        }
    }
}
