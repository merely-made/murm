//! Bounded PKI wire experiment under real scoped custody. Default is receive
//! only. Optional `reply` consumes a durable ID, sends one PKI text and captures
//! an unclassified stock control response; it does not assert ACK semantics.
use station_custody::DjinnStationAuthority;
use sennet::{
    pki::LocalIdentity,
    transport::{Header, Packet},
};
use signalman::radio_comms::sennet_ids::SennetPacketIds;
use signalman_desktop::resident::sennet_station;
use std::{path::Path, time::Duration};
use tulle::{airtime::AirtimeBudget, direct_phy_serial::DirectPhySerialLink};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 4 || (args.len() == 5 && matches!(args[4].as_str(), "reply" | "ack-reply")))
    {
        return Err("usage: sennet_pki_capture <public-config> <stock-public-key-hex> <expected-bench-text> <new-output> [reply]".into());
    }
    let settings = sennet_station::load(Path::new(&args[0]))?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    if !settings.realm.starts_with("bench-") || !args[2].starts_with("sennet-pki-") {
        return Err("require explicit bounded bench realm and tag".into());
    }
    let output = Path::new(&args[3]);
    let ready = output.with_extension("ready");
    if output.exists() || ready.exists() {
        return Err("preserve previous capture".into());
    }
    let device = serde_json::from_value(value["device_id"].clone())?;
    let peer = u32::try_from(value["peer_node"].as_u64().ok_or("missing peer")?)?;
    let public: [u8; 32] = hex::decode(&args[1])?
        .try_into()
        .map_err(|_| "require full stock public key")?;
    let released = DjinnStationAuthority::default().open_sennet(device, &settings.realm)?;
    let source = released.identity.node_number();
    if source == peer {
        return Err("known node collision".into());
    }
    let identity = LocalIdentity::from_secret(*released.material.seed_bytes());
    let pair = identity
        .peer(source, peer, public)
        .map_err(|e| format!("{e:?}"))?;
    let reservation = if args.len() == 5 {
        let path = value["packet_ids"]
            .as_str()
            .ok_or("missing packet-ID path")?;
        let count = u32::try_from(
            value["reservation_size"]
                .as_u64()
                .ok_or("missing reservation size")?,
        )?;
        Some(SennetPacketIds::reserve(
            Path::new(path),
            &settings.realm,
            source,
            count,
        )?)
    } else {
        None
    };
    let acknowledge = args.get(4).is_some_and(|mode| mode == "ack-reply");
    let ack_id = reservation
        .as_ref()
        .filter(|_| acknowledge)
        .map(|(_, lease)| lease.next());
    let reply_id = reservation
        .as_ref()
        .map(|(_, lease)| lease.next() + u32::from(acknowledge));
    if reservation
        .as_ref()
        .is_some_and(|(_, lease)| reply_id.unwrap() >= lease.end())
    {
        return Err("nonce interval too short for ACK/reply".into());
    }
    let mut report = serde_json::json!({"result":"not_completed","transmitted":false,"authority":"djinn","node_number":source,"pki_public_key":hex::encode(identity.public_key()),"peer_node":peer,"peer_public_key":args[1],"frames":[],"nonce_placement":"LE32(packet_id), extra nonce(4), LE32(source), zero(1), observed on 2.7.26","reply_packet_id":reply_id});
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result=rt.block_on(async {
        released.lease.check()?;
        let mut radio=DirectPhySerialLink::open(&settings.port,settings.profile,
            AirtimeBudget::new(settings.airtime_window_ms,settings.airtime_allowance_ms),settings.serial.clone())?;
        let captured=async {
            radio.wait_online().await?;
            released.lease.check()?;
            std::fs::write(&ready,serde_json::to_vec_pretty(&report)?)?;
            let until=tokio::time::Instant::now()+Duration::from_secs(90);
            let mut sent_reply=false;
            while tokio::time::Instant::now()<until {
                released.lease.check()?;
                let Ok(Some(rx))=tokio::time::timeout(Duration::from_millis(100),radio.recv()).await else { continue; };
                let Ok(mut packet)=Packet::decode(&rx.frame) else { continue; };
                let Header {source:from,destination,channel_hash,..}=packet.header;
                if from!=peer || destination!=source { continue; }
                if report["frames"].as_array().unwrap().len()>=32 { return Err("bounded capture full".into()); }
                if sent_reply && channel_hash==8 {
                    packet.apply_channel_cipher(&sennet::transport::ChannelKey::Aes128([
                        0xd4,0xf1,0xbb,0x3a,0x20,0x29,0x07,0x59,0xf0,0xbc,0xff,0xab,0xcf,0x4e,0x69,0x01]));
                    let response=sennet::routing::Response::decode(&packet.payload).ok().filter(|r|Some(r.request_id)==reply_id);
                    report["frames"].as_array_mut().unwrap().push(serde_json::json!({"frame_hex":hex::encode(&rx.frame),"channel_application_hex":hex::encode(&packet.payload),"matched_routing_code":response.map(|r|r.code),"authenticated":false}));
                    if let Some(response)=response {
                        if !response.is_success(){return Err("stock routing response is nonzero, no success ACK".into());}
                        report["result"]="matched_stock_routing_success_not_native_gate".into();
                        return Ok::<_,Box<dyn std::error::Error>>(());
                    }
                    continue;
                }
                if channel_hash!=0 {continue;}
                let mut observed=serde_json::json!({"frame_hex":hex::encode(&rx.frame),"authenticated_candidate":false});
                    if let Ok((_,plain))=pair.open_packet(&rx.frame) {
                        observed["authenticated_candidate"]=true.into();
                        observed["application_hex"]=hex::encode(&plain).into();
                        if sennet::application::decode_text(&plain)==Ok(args[2].as_str()) {
                            observed["expected_text_match"]=true.into();
                            report["frames"].as_array_mut().unwrap().push(observed);
                            if let Some(packet_id)=reply_id {
                                if !sent_reply {
                                    if let Some(packet_id)=ack_id {
                                        let ack=sennet::routing::encode_ack(packet.header.packet_id).map_err(|e|format!("{e:?}"))?;
                                        let mut ack=sennet::transport::Packet {header:Header{source,destination:peer,packet_id,hop_limit:3,want_ack:false,via_mqtt:false,hop_start:3,channel_hash:8,next_hop:0,relay_node:source as u8},payload:ack};
                                        ack.apply_channel_cipher(&sennet::transport::ChannelKey::Aes128([0xd4,0xf1,0xbb,0x3a,0x20,0x29,0x07,0x59,0xf0,0xbc,0xff,0xab,0xcf,0x4e,0x69,0x01]));
                                        let ack=ack.encode().map_err(|e|format!("{e:?}"))?;
                                        released.lease.check()?;released.lease.check_live()?;
                                        let duration=radio.send(ack.clone()).await?;
                                        report["transmitted"]=true.into();
                                        report["ack_packet_id"]=packet_id.into();
                                        report["ack_frame_hex"]=hex::encode(ack).into();
                                        report["ack_physical_airtime_ms"]=(duration.as_secs_f64()*1000.0).into();
                                    }
                                    let mut extra=[0;4];getrandom::fill(&mut extra).map_err(|e|format!("extra nonce randomness: {e}"))?;
                                    let body=sennet::application::encode_text(&format!("{}-return",args[2])).map_err(|e|format!("{e:?}"))?;
                                    let header=Header {source,destination:peer,packet_id,hop_limit:3,want_ack:true,via_mqtt:false,hop_start:3,channel_hash:0,next_hop:0,relay_node:source as u8};
                                    let frame=pair.seal_packet(header,extra,&body).map_err(|e|format!("{e:?}"))?;
                                    released.lease.check()?;released.lease.check_live()?;
                                    let duration=radio.send(frame.clone()).await?;
                                    report["transmitted"]=true.into();report["reply_frame_hex"]=hex::encode(frame).into();
                                    report["pki_reply_transmitted"]=true.into();
                                    report["physical_airtime_ms"]=(duration.as_secs_f64()*1000.0).into();
                                    sent_reply=true;
                                }
                                continue;
                            } else {
                                report["result"]="matched_stock_pki_text".into();
                                return Ok::<_,Box<dyn std::error::Error>>(());
                            }
                        }
                    }
                report["frames"].as_array_mut().unwrap().push(observed);
            }
            Err("no authenticated matching stock text before deadline".into())
        }.await;
        let closed=radio.shutdown().await.map_err(|e| -> Box<dyn std::error::Error> {Box::new(e)});
        captured.and(closed)
    });
    if let Err(error) = &result {
        report["result"] = "failed".into();
        report["error"] = error.to_string().into();
    }
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    result
}
