//! The transit duplicate memory, against both generation containers.

use super::*;
use crate::transport::memory::Generation;

const BOARD: MemoryProfile = MemoryProfile {
    capacity: 2,
    max_age: Some(60),
};
const DESKTOP: MemoryProfile = MemoryProfile {
    capacity: 2,
    max_age: None,
};

/// Run a body over the bounded array and, with `std`, the set. One that takes a capacity
/// also runs at each driver's own: the board's per generation, and the desktop's.
macro_rules! both_generations {
    ($name:ident) => {
        mod $name {
            use super::*;

            #[test]
            fn array() {
                super::$name::<heapless::Vec<AddressHash, 2>>();
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn set() {
                super::$name::<std::collections::HashSet<AddressHash>>();
            }
        }
    };
    ($name:ident, sized) => {
        mod $name {
            use super::*;

            #[test]
            fn array() {
                super::$name::<heapless::Vec<AddressHash, 2>>(2);
            }

            #[cfg(feature = "alloc")]
            #[test]
            fn board() {
                use crate::node::TRANSPORT_DEDUP_HASHES;
                super::$name::<heapless::Vec<AddressHash, TRANSPORT_DEDUP_HASHES>>(
                    TRANSPORT_DEDUP_HASHES,
                );
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn set() {
                super::$name::<std::collections::HashSet<AddressHash>>(2);
            }

            #[cfg(feature = "tokio")]
            #[test]
            fn desktop() {
                super::$name::<std::collections::HashSet<AddressHash>>(
                    crate::endpoint::PACKET_HASHES,
                );
            }
        }
    };
}

both_generations!(a_full_generation_becomes_the_previous_one, sized);
fn a_full_generation_becomes_the_previous_one<G: Generation>(capacity: usize) {
    let profile = MemoryProfile {
        capacity,
        ..DESKTOP
    };
    let mut memory = TransitMemory::<G, u64>::new(profile, 0);
    assert!(memory.insert(key(1), 0));
    assert!(!memory.insert(key(1), 0), "a repeat");
    for n in 2..=capacity {
        assert!(memory.insert(nth(n), 0));
    }
    assert!(
        memory.insert(nth(capacity + 1), 0),
        "turns the full generation over"
    );
    assert!(
        !memory.insert(key(1), 0),
        "the previous generation still counts"
    );
    for n in capacity + 2..=2 * capacity {
        assert!(memory.insert(nth(n), 0));
    }
    assert!(
        memory.insert(nth(2 * capacity + 1), 0),
        "turns over again"
    );
    assert!(
        memory.insert(key(1), 0),
        "two generations back is forgotten"
    );
    assert!(
        !memory.insert(key(1), u64::MAX),
        "no time limit without a max age"
    );
}

both_generations!(time_turns_a_generation_over_and_twice_it_clears_both);
fn time_turns_a_generation_over_and_twice_it_clears_both<G: Generation>() {
    let mut memory = TransitMemory::<G, u64>::new(BOARD, 0);
    assert!(memory.insert(key(1), 10));
    assert!(
        !memory.insert(key(1), 60),
        "turned over at 60, still remembered"
    );
    assert!(
        memory.current.len() == 0,
        "the check found it in the previous generation"
    );
    assert!(memory.insert(key(2), 100));
    assert!(
        memory.insert(key(1), 180),
        "120 after the turn-over clears both"
    );
    assert!(memory.insert(key(2), 181));
}

// The endpoint turned a generation over as it filled, the board as the next hash arrived.
// Both answer every insert alike, so either serves both drivers.
// Keys and inserts scale with capacity: at 2, 500 inserts over 6 keys. Past a byte of keys
// a tenth as many inserts per slot still turns over about twelve times, in under two seconds.
both_generations!(turning_over_late_answers_as_turning_over_early, sized);
fn turning_over_late_answers_as_turning_over_early<G: Generation>(capacity: usize) {
    use std::collections::HashSet;
    let profile = MemoryProfile {
        capacity,
        ..DESKTOP
    };
    let mut memory = TransitMemory::<G, u64>::new(profile, 0);
    let (mut current, mut previous) = (HashSet::new(), HashSet::new());
    let mut turned = 0_usize;
    let mut eager = |hash: AddressHash| {
        if previous.contains(&hash) || current.contains(&hash) {
            return false;
        }
        current.insert(hash);
        if current.len() >= capacity {
            previous = core::mem::take(&mut current);
            turned += 1;
        }
        true
    };
    let keys = 3 * capacity;
    let (mut state, mut high) = (7_u32, 11_u32);
    let per_slot = if keys > 256 { 25 } else { 250 };
    for _ in 0..per_slot * capacity {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        // Up to a byte of keys draws one byte, as at 2; more draws two more from a second stream.
        let mut draw = usize::from((state >> 16) as u8);
        if keys > 256 {
            high = high.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            draw += 256 * (high >> 16) as usize;
        }
        let hash = nth(draw % keys);
        assert_eq!(memory.insert(hash, 0), eager(hash));
    }
    assert!(turned >= 8, "{turned} turn-overs at {capacity}");
}
