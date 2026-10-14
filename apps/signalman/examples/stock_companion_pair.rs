//! Finite stock-radio Comms bench. No flashing, key import or radio configuration.
//! Probe: stock_companion_pair probe <meshtastic|meshcore> <port> <realm> <new-output-dir> [channel]
//! Pair:  stock_companion_pair pair <meshtastic|meshcore> <left-port> <right-port> <realm> <new-output-dir> [channel]
//! Pair requires two already configured stock radios with compatible profiles.
//! MeshCore contacts must already contain each other's full public keys.
use signalman::{
    comms_adapter::comms::{DeliveryStatus, Direction, Draft, MessageId},
    radio_comms::{
        FileJournal, RadioComms, RadioRecord,
        companion::{CompanionProtocol, CompanionSession},
    },
};
use std::{
    error::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type Session = CompanionSession<serial2_tokio::SerialPort>;
struct Radio {
    session: Session,
    #[cfg(unix)]
    _exclusive: Exclusive,
}
#[cfg(unix)]
struct Exclusive(std::fs::File);
#[cfg(unix)]
impl Drop for Exclusive {
    fn drop(&mut self) {
        let _ = rustix::termios::ioctl_tiocnxcl(&self.0);
    }
}
async fn connect(
    port: &str,
    protocol: CompanionProtocol,
    realm: &str,
    journal: PathBuf,
) -> Result<Radio, Box<dyn Error>> {
    let serial = serial2_tokio::SerialPort::open(port, 115_200)?;
    #[cfg(unix)]
    let exclusive = {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(port)?;
        rustix::termios::ioctl_tiocexcl(&file)?;
        Exclusive(file)
    };
    serial.set_rts(false)?;
    serial.set_dtr(true)?;
    let session =
        CompanionSession::connect(serial, protocol, realm.into(), FileJournal::new(journal))
            .await?;
    Ok(Radio {
        session,
        #[cfg(unix)]
        _exclusive: exclusive,
    })
}
fn address(owner: &RadioComms) -> String {
    owner.model().identities()[0].address.clone()
}
async fn exchange(
    left: &mut Radio,
    right: &mut Radio,
    protocol: CompanionProtocol,
    text: &str,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let sender = left.session.owner();
    let receiver = right.session.owner();
    let source = address(&sender);
    let destination = address(&receiver);
    let mut draft = Draft::reply_to(sender.conversation(protocol.kind(), &destination)?);
    draft.body = text.into();
    let id: MessageId = sender.model().send(&draft).await?;
    let started = Instant::now();
    loop {
        let (a, b) = tokio::join!(left.session.poll(), right.session.poll());
        a?;
        b?;
        let sent = sender
            .records()?
            .into_iter()
            .find(|r| r.message.id == id)
            .ok_or("persisted send vanished")?;
        let received: Vec<RadioRecord> = receiver
            .records()?
            .into_iter()
            .filter(|r| {
                r.message.direction == Direction::Incoming
                    && r.message.author.address == source
                    && r.message.body.text() == text
            })
            .collect();
        if let DeliveryStatus::Failed { detail } = sent.delivery {
            return Err(format!("device send refused: {detail}").into());
        }
        if received.len() > 1 {
            return Err("matching radio text appeared more than once".into());
        }
        if sent.delivery == DeliveryStatus::ProtocolAcknowledged && received.len() == 1 {
            return Ok(
                serde_json::json!({"source":source,"destination":destination,"text":text,"elapsed_ms":started.elapsed().as_millis(),"sent":sent,"received":received[0],"recipient_text_match":true,"matched_protocol_ack":true}),
            );
        }
        if started.elapsed() > Duration::from_secs(180) {
            return Err(format!(
                "radio exchange timed out: receive matches {}, delivery {:?}",
                received.len(),
                sent.delivery
            )
            .into());
        }
    }
}
fn snapshot(radio: &Radio, tag: Option<&str>) -> Result<serde_json::Value, Box<dyn Error>> {
    let owner = radio.session.owner();
    let records = owner.records()?;
    let count = records.len();
    let selected: Vec<_> = records
        .into_iter()
        .filter(|r| tag.is_some_and(|tag| r.message.body.text().starts_with(tag)))
        .collect();
    Ok(
        serde_json::json!({"identity":address(&owner),"diagnostics":owner.diagnostics()?,"total_record_count":count,"bench_records":selected}),
    )
}
fn check_history(owner: &RadioComms, before: &[RadioRecord]) -> Result<(), Box<dyn Error>> {
    let after = owner.records()?;
    for previous in before {
        if !after.iter().any(|current| {
            current.message.id == previous.message.id
                && current.message.body == previous.message.body
                && current.conversation == previous.conversation
                && current.message.author == previous.message.author
                && current.message.timestamp_ms == previous.message.timestamp_ms
                && current.delivery == previous.delivery
                && current.facts == previous.facts
        }) {
            return Err("reconnect lost or changed durable message history".into());
        }
    }
    if after
        .iter()
        .filter(|r| r.message.direction == Direction::Outgoing)
        .count()
        != before
            .iter()
            .filter(|r| r.message.direction == Direction::Outgoing)
            .count()
    {
        return Err("reconnect created an outgoing intent".into());
    }
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args
        .first()
        .ok_or("supply probe or pair; no default ports are opened")?;
    let pair = match mode.as_str() {
        "pair" => true,
        "probe" => false,
        _ => return Err("mode must be probe or pair".into()),
    };
    let required = if pair { 6 } else { 5 };
    if args.len() < required || args.len() > required + 1 {
        return Err("see example's probe/pair usage; explicit ports, realm and new output directory required".into());
    }
    let channel = args
        .get(required)
        .map(|s| s.parse::<u8>())
        .transpose()?
        .unwrap_or(0);
    if channel > 7 {
        return Err("channel index must be 0..7".into());
    }
    let protocol = match args[1].as_str() {
        "meshtastic" => CompanionProtocol::Meshtastic { channel },
        "meshcore" => CompanionProtocol::MeshCore,
        _ => return Err("protocol must be meshtastic or meshcore".into()),
    };
    let left_port = &args[2];
    let right_port = if pair { Some(args[3].as_str()) } else { None };
    if right_port.is_some_and(|right| left_port.eq_ignore_ascii_case(right)) {
        return Err("pair ports must differ".into());
    }
    let realm = &args[required - 2];
    if realm.is_empty()
        || realm.len() > 128
        || !realm
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("invalid realm name".into());
    }
    let output = Path::new(&args[required - 1]);
    std::fs::create_dir(output)?; // Fresh evidence; never overwrite an earlier trial.
    let mut receipt = serde_json::json!({"schema":1,"protocol":args[1],"ports":if pair{vec![left_port.as_str(),right_port.unwrap()]}else{vec![left_port.as_str()]},"realm":realm,"channel":channel,"mode":mode,"result":"not_completed","exchanges":[],"firmware_or_configuration_mutated":false});
    let result = async {
        let left_path = output.join("left.json");
        let mut left = connect(left_port, protocol, realm, left_path.clone()).await?;
        receipt["left"] = snapshot(&left, None)?;
        let Some(right_port) = right_port else {
            return Ok::<(), Box<dyn Error>>(());
        };
        let right_path = output.join("right.json");
        let mut right = connect(right_port, protocol, realm, right_path.clone()).await?;
        if address(&left.session.owner()) == address(&right.session.owner()) {
            return Err("pair returned the same public identity".into());
        }
        let tag = format!("signalman-stock-{}-{}", args[1], getrandom::u32().map_err(|error| std::io::Error::other(error.to_string()))?);
        for generation in 0..2 {
            for reverse in [false, true] {
                let text = format!("{tag}-generation-{generation}-reverse-{reverse}");
                println!("RF exchange {text}");
                let exchange = if reverse {
                    exchange(&mut right, &mut left, protocol, &text).await?
                } else {
                    exchange(&mut left, &mut right, protocol, &text).await?
                };
                receipt["exchanges"].as_array_mut().unwrap().push(exchange);
            }
            if generation == 0 {
                receipt["before_reconnect"] =
                    serde_json::json!({"left":snapshot(&left, Some(&tag))?,"right":snapshot(&right, Some(&tag))?});
                let left_history = left.session.owner().records()?;
                let right_history = right.session.owner().records()?;
                drop(left);
                drop(right);
                left = connect(left_port, protocol, realm, left_path.clone()).await?;
                right = connect(right_port, protocol, realm, right_path.clone()).await?;
                check_history(&left.session.owner(), &left_history)?;
                check_history(&right.session.owner(), &right_history)?;
                receipt["reconnect_history_preserved"] = true.into();
            }
        }
        receipt["left"] = snapshot(&left, Some(&tag))?;
        receipt["right"] = snapshot(&right, Some(&tag))?;
        Ok(())
    }
    .await;
    match &result {
        Ok(()) => receipt["result"] = "passed".into(),
        Err(e) => {
            receipt["result"] = "failed".into();
            receipt["error"] = e.to_string().into();
        }
    }
    std::fs::write(
        output.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    result
}
