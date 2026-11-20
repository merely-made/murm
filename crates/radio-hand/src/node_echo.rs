//! The node channel's loopback service, as a per-board setting (wall node D5: "Off on V4,
//! per-board setting"). What arrives whole goes back whole on the same link when the board
//! chose [`Echo::On`]; with [`Echo::Off`] a completed resource is only a report. Beside it,
//! the per-board limit on resources addressed to the node (D8: "Refuse all, per-board limit"):
//! [`limit_inbound`] at 0 refuses every offer at its advertisement, before any reassembly.
//!
//! Radio-free, so the decision is tested on the desk under `replay`; the boards reach it
//! through `node`, which implies `replay`.

use retinue::hash::AddressHash;
use retinue::node::{Actions, InterfaceId, Node};
use retinue::resource::RANDOM_HASH_LEN;
use retinue::token::IV_LEN;

/// Whether a board echoes completed inbound resources. Chosen by each firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Echo {
    On,
    Off,
}

/// Refuse inbound resources larger than `max_size` bytes, metadata included; 0 refuses all.
/// The sender gets a cancel and the node counts a refused offer.
pub fn limit_inbound<const PEERS: usize, const ACTIONS: usize, const LINKS: usize>(
    node: &mut Node<PEERS, ACTIONS, LINKS>,
    max_size: usize,
) {
    node.set_max_inbound_resource(max_size);
}

/// What became of one completed inbound resource.
#[derive(Debug)]
pub enum EchoOutcome<const ACTIONS: usize> {
    /// The board does not echo: no entropy drawn, no transfer started.
    Off,
    /// The board could not produce the transfer's random hash and IV.
    Unseeded,
    /// The link vanished or a transfer still holds it.
    Refused,
    /// The echo's transfer started; these are its first sends.
    Started(Actions<ACTIONS>),
}

/// Answer one completed resource. `entropy` is asked only when the board echoes.
#[allow(clippy::too_many_arguments)]
pub fn echo_resource<const PEERS: usize, const ACTIONS: usize, const LINKS: usize>(
    echo: Echo,
    node: &mut Node<PEERS, ACTIONS, LINKS>,
    interface: InterfaceId,
    link_id: AddressHash,
    data: &[u8],
    metadata: Option<&[u8]>,
    entropy: impl FnOnce() -> Option<([u8; RANDOM_HASH_LEN], [u8; IV_LEN])>,
    now: u64,
) -> EchoOutcome<ACTIONS> {
    if echo == Echo::Off {
        return EchoOutcome::Off;
    }
    let Some((random_hash, iv)) = entropy() else {
        return EchoOutcome::Unseeded;
    };
    let started = match metadata {
        Some(metadata) => {
            node.publish_with_metadata(link_id, interface, data, metadata, random_hash, &iv, now)
        }
        None => node.publish(link_id, interface, data, random_hash, &iv, now),
    };
    match started {
        Some(actions) => EchoOutcome::Started(actions),
        None => EchoOutcome::Refused,
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;
    use retinue::destination::DestinationName;
    use retinue::identity::PrivateIdentity;

    fn node() -> Node<32, 8, 4> {
        Node::new(
            PrivateIdentity::from_secret_bytes(&[0x42; 64]),
            DestinationName::new("retinue", ["node"]).name_hash(),
        )
    }

    fn answer(echo: Echo, asked: &Cell<u8>) -> EchoOutcome<8> {
        echo_resource(
            echo,
            &mut node(),
            0,
            AddressHash::from_bytes([7; 16]),
            b"whole resource",
            Some(b"meta"),
            || {
                asked.set(asked.get() + 1);
                Some(([1; RANDOM_HASH_LEN], [2; IV_LEN]))
            },
            1_000,
        )
    }

    #[test]
    fn echo_off_draws_no_entropy_and_starts_nothing() {
        let asked = Cell::new(0);
        assert!(matches!(answer(Echo::Off, &asked), EchoOutcome::Off));
        assert_eq!(asked.get(), 0);
    }

    #[test]
    fn echo_on_draws_entropy_and_tries_to_publish_on_the_link() {
        let asked = Cell::new(0);
        // No such link on a fresh node: the publish is attempted and refused.
        assert!(matches!(answer(Echo::On, &asked), EchoOutcome::Refused));
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn echo_on_without_entropy_is_unseeded() {
        let outcome = echo_resource(
            Echo::On,
            &mut node(),
            0,
            AddressHash::from_bytes([7; 16]),
            b"whole resource",
            None,
            || None,
            1_000,
        );
        assert!(matches!(outcome, EchoOutcome::<8>::Unseeded));
    }

    /// Two nodes with a link from `a` to `b`, built through Retinue's public API only.
    fn linked() -> (Node<32, 8, 4>, Node<32, 8, 4>, AddressHash) {
        use retinue::announce::{AnnounceBlob, RAND_HASH_LEN};
        use retinue::node::Action;
        let sent = |actions: &Actions<8>| {
            actions.iter().find_map(|action| match action {
                Action::Send { packet, .. } => Some(packet.clone()),
                _ => None,
            })
        };
        let mut a = Node::new(
            PrivateIdentity::from_secret_bytes(&[0x11; 64]),
            DestinationName::new("retinue", ["a"]).name_hash(),
        );
        let mut b = Node::new(
            PrivateIdentity::from_secret_bytes(&[0x22; 64]),
            DestinationName::new("retinue", ["b"]).name_hash(),
        );
        let blob = AnnounceBlob::from_wire([2; RAND_HASH_LEN]);
        a.ingest(0, &b.announce(&blob, None), 0);
        let request = sent(&a.open_link(b.destination(), 0, &[0x31; 64], 0).unwrap()).unwrap();
        let proof = sent(&b.ingest(0, &request, 0)).unwrap();
        let up = a.ingest(0, &proof, 0);
        let id = up
            .iter()
            .find_map(|action| match action {
                Action::LinkUp { link_id } => Some(*link_id),
                _ => None,
            })
            .expect("link up");
        let rtt = sent(&up).unwrap();
        b.ingest(0, &rtt, 0);
        (a, b, id)
    }

    /// `a` offers `b` a resource; what `b` says back.
    fn offer(limit: Option<usize>) -> (Node<32, 8, 4>, AddressHash, Actions<8>) {
        let (mut a, mut b, id) = linked();
        if let Some(limit) = limit {
            limit_inbound(&mut b, limit);
        }
        let started = a
            .publish(
                id,
                0,
                b"a resource for the relay",
                [1; RANDOM_HASH_LEN],
                &[2; IV_LEN],
                0,
            )
            .expect("transfer starts");
        let advertisement = started
            .iter()
            .find_map(|action| match action {
                retinue::node::Action::Send { packet, .. } => Some(packet.clone()),
                _ => None,
            })
            .expect("advertisement");
        let answer = b.ingest(0, &advertisement, 0);
        (b, id, answer)
    }

    #[test]
    fn a_zero_inbound_limit_refuses_the_offer_and_counts_it() {
        let (b, id, answer) = offer(Some(0));
        assert_eq!(b.refused_offers(), 1);
        assert!(!b.transfer_active(id), "no reassembly state is held");
        assert!(
            answer
                .iter()
                .any(|action| matches!(action, retinue::node::Action::Send { .. })),
            "the sender is told to stop"
        );
    }

    #[test]
    fn retinue_s_default_limit_accepts_the_same_offer() {
        let (b, id, _) = offer(Some(retinue::resource_transfer::DEFAULT_MAX_RESOURCE_SIZE));
        assert_eq!(b.refused_offers(), 0);
        assert!(b.transfer_active(id), "the transfer is under way");
        let (unset, id, _) = offer(None);
        assert_eq!(unset.refused_offers(), 0);
        assert!(unset.transfer_active(id));
    }
}
