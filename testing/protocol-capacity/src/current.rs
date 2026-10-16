//! Current retained-state fixture. Synthetic keys, no RF or durable writes.
use alloc::{vec, vec::Vec};
use core::{mem::size_of, time::Duration};
use sennet::{
    flood::{ManagedFloodConfig, RelayDelayWindow},
    instance::{PacketIdLease, ReceiveOutcome, SennetInstance, SennetInstanceConfig},
    node_info::NodeDirectoryConfig,
    pki::LocalIdentity,
    transport::Header,
};
use tucket::instance::{Instance, InstanceConfig, SendTiming};

/// Explicit probe choices, not firmware operating defaults.
pub const PRIVATE_SEEN: usize = 64;
pub const PRIVATE_TEXT_BYTES: usize = sennet::pki::MAX_PLAINTEXT - 5;

pub struct Residents {
    pub retinue: super::RetinueNode,
    pub sennet: SennetInstance,
    pub tucket: Instance,
}

/// Target ABI sizes, heap excluded. The final value is the actual radio runtime
/// including its work/report queues, measured separately from these objects.
#[unsafe(no_mangle)]
pub static PROTOCOL_LAYOUT_V2: [u32; 6] = [
    size_of::<usize>() as u32,
    size_of::<super::RetinueNode>() as u32,
    size_of::<SennetInstance>() as u32,
    size_of::<Instance>() as u32,
    size_of::<Residents>() as u32,
    size_of::<radio_hand::instances::Runtime>() as u32,
];

fn header(source: u32, destination: u32, packet_id: u32) -> Header {
    Header {
        source,
        destination,
        packet_id,
        hop_limit: 3,
        want_ack: true,
        via_mqtt: false,
        hop_start: 3,
        channel_hash: 0,
        next_hop: 0,
        relay_node: source as u8,
    }
}

fn info(number: u8) -> Vec<u8> {
    let mut user = Vec::new();
    for (tag, len) in [(1, 32), (2, 64), (3, 16)] {
        sennet::protobuf::write_tag(tag, 2, &mut user);
        sennet::protobuf::write_varint(len, &mut user);
        user.extend(core::iter::repeat_n(b'x', len as usize));
    }
    let mut node = vec![8, number];
    sennet::protobuf::write_tag(2, 2, &mut node);
    sennet::protobuf::write_varint(user.len() as u64, &mut node);
    node.extend(user);
    let mut outer = vec![34];
    sennet::protobuf::write_varint(node.len() as u64, &mut outer);
    outer.extend(node);
    outer
}

/// Full baseline Retinue/contact state, maximum private frames, dedup churn,
/// malformed/refused inputs, and coexisting logical ACK and physical work.
/// Temporary synthetic peers count toward the peak, and no radio is used.
pub fn workload() -> Residents {
    workload_with_checkpoint(|| {})
}

/// Same workload with a bounded supervisor checkpoint between private frames.
/// This does not poll USB/radio tasks or qualify service scheduling under load.
pub fn workload_with_checkpoint(checkpoint: fn()) -> Residents {
    checkpoint();
    let super::Residents {
        retinue,
        channel,
        tucket,
        ids,
        flood,
        directory,
        pending,
    } = super::workload_with_checkpoint(checkpoint);
    checkpoint();
    drop((ids, flood, directory, pending));
    let mut tucket = Instance::new(tucket, InstanceConfig::new(2, 1000).unwrap());
    let peers: Vec<_> = (0..=u8::MAX)
        .filter(|&h| tucket.node().contact(h).is_some())
        .take(2)
        .collect();
    assert_eq!(peers.len(), 2);
    for peer in peers {
        tucket
            .begin_send(
                0,
                peer,
                "x".repeat(171),
                Default::default(),
                SendTiming {
                    timestamp: 1,
                    expires_at: 15000,
                    allowed_until: 15000,
                },
            )
            .unwrap();
    }
    assert_eq!(tucket.pending_len(), 2);
    let mut sennet = SennetInstance::new(
        SennetInstanceConfig {
            channel,
            flood: ManagedFloodConfig {
                channel_hash: 8,
                relay_node: 11,
                seen_capacity: 32,
                delay: RelayDelayWindow::new(Duration::ZERO, Duration::ZERO).unwrap(),
            },
            directory: NodeDirectoryConfig {
                capacity: 8,
                id_limit: 32,
                long_name_limit: 64,
                short_name_limit: 16,
            },
            pending_ttl: 15000,
        },
        PacketIdLease::new(11, 1, 4097, 1).unwrap(),
    )
    .unwrap();
    let local = LocalIdentity::from_secret([7; 32]);
    let remote = LocalIdentity::from_secret([9; 32]);
    let peer = remote.peer(22, 11, local.public_key()).unwrap();
    sennet
        .install_pki(22, local, remote.public_key(), 15000, PRIVATE_SEEN)
        .unwrap();
    for number in 1..=8 {
        assert!(sennet.ingest_node_info(0, &info(number)).unwrap());
    }
    assert_eq!(sennet.directory().len(), 8);
    let text = "x".repeat(PRIVATE_TEXT_BYTES);
    let plain = sennet::application::encode_text(&text).unwrap();
    assert_eq!(plain.len(), sennet::pki::MAX_PLAINTEXT);
    let mut last = Vec::new();
    for n in 0..(PRIVATE_SEEN * 4) {
        checkpoint();
        let now = n as u64 * 10;
        last = peer
            .seal_packet(header(22, 11, 1000 + n as u32), [n as u8; 4], &plain)
            .unwrap();
        assert!(matches!(
            sennet.receive(now, &last).unwrap(),
            ReceiveOutcome::PkiText(_)
        ));
        let ack = sennet.take_outbound(now + 1).unwrap().unwrap();
        sennet
            .complete_tx(now + 2, ack.operation_id, ack.identity)
            .unwrap();
        assert!(matches!(
            sennet.receive(now + 3, &last).unwrap(),
            ReceiveOutcome::Duplicate { .. }
        ));
        let ack = sennet.take_outbound(now + 4).unwrap().unwrap();
        sennet
            .complete_tx(now + 5, ack.operation_id, ack.identity)
            .unwrap();
    }
    let now = PRIVATE_SEEN as u64 * 40;
    checkpoint();
    let id = sennet
        .queue_directed_text(now, header(0, 22, 0), &text, [0xa5; 4])
        .unwrap();
    let outgoing = sennet.take_outbound(now + 1).unwrap().unwrap();
    assert_eq!(peer.open_packet(&outgoing.frame).unwrap().1, plain);
    sennet
        .complete_tx(now + 2, outgoing.operation_id, id)
        .unwrap();
    assert!(sennet.awaiting_ack_operation_id().is_some());
    assert!(matches!(
        sennet.receive(now + 3, &last).unwrap(),
        ReceiveOutcome::Duplicate { .. }
    ));
    // A queued control ACK coexists with the outgoing logical response window.
    assert!(sennet.pending_identity().is_some());
    let mut corrupt = last;
    corrupt[16] ^= 1;
    assert!(sennet.receive(now + 4, &corrupt).is_err());
    assert!(
        sennet
            .queue_directed_text(now + 5, header(0, 22, 0), "busy", [1; 4])
            .is_err()
    );
    Residents {
        retinue,
        sennet,
        tucket,
    }
}
