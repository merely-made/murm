//! The transit duplicate memory, against both generation containers.

use super::*;

const BOARD: MemoryProfile = MemoryProfile {
    capacity: 2,
    max_age: Some(60),
};
const DESKTOP: MemoryProfile = MemoryProfile {
    capacity: 2,
    max_age: None,
};

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
}

both_generations!(a_full_generation_becomes_the_previous_one);
fn a_full_generation_becomes_the_previous_one<G: Generation>() {
    let mut memory = TransitMemory::<G, u64>::new(DESKTOP, 0);
    assert!(memory.insert(key(1), 0));
    assert!(!memory.insert(key(1), 0), "a repeat");
    assert!(memory.insert(key(2), 0));
    assert!(memory.insert(key(3), 0), "turns the full generation over");
    assert!(
        !memory.insert(key(1), 0),
        "the previous generation still counts"
    );
    assert!(memory.insert(key(4), 0));
    assert!(memory.insert(key(5), 0), "turns over again");
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
        memory.current.is_empty(),
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
both_generations!(turning_over_late_answers_as_turning_over_early);
fn turning_over_late_answers_as_turning_over_early<G: Generation>() {
    let mut memory = TransitMemory::<G, u64>::new(DESKTOP, 0);
    let (mut current, mut previous) = (Vec::new(), Vec::new());
    let mut eager = |hash: AddressHash| {
        if previous.contains(&hash) || current.contains(&hash) {
            return false;
        }
        current.push(hash);
        if current.len() >= DESKTOP.capacity {
            previous = core::mem::take(&mut current);
        }
        true
    };
    let mut state = 7_u32;
    for _ in 0..500 {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let hash = key((state >> 16) as u8 % 6);
        assert_eq!(memory.insert(hash, 0), eager(hash));
    }
}
