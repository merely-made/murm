#![cfg(feature = "tucket-contact-store")]
use radio_hand::{
    store::{self, Slot},
    tucket_contacts::*,
};

#[derive(Clone)]
struct Flash {
    pages: [[u8; SLOT_LEN]; 2],
    mutations: usize,
    cut_after_words: Option<usize>,
    reads: usize,
    refuse_read: Option<usize>,
}
impl Default for Flash {
    fn default() -> Self {
        Self {
            pages: [[0xff; SLOT_LEN]; 2],
            mutations: 0,
            cut_after_words: None,
            reads: 0,
            refuse_read: None,
        }
    }
}
fn index(slot: Slot) -> usize {
    if slot == Slot::A { 0 } else { 1 }
}
impl Storage for Flash {
    type Error = ();
    fn read(&mut self, slot: Slot, out: &mut [u8; SLOT_LEN]) -> Result<(), ()> {
        self.reads += 1;
        if self.refuse_read == Some(self.reads) {
            return Err(());
        }
        *out = self.pages[index(slot)];
        Ok(())
    }
    fn erase(&mut self, slot: Slot) -> Result<(), ()> {
        self.mutations += 1;
        self.pages[index(slot)].fill(0xff);
        Ok(())
    }
    fn write(&mut self, slot: Slot, offset: usize, bytes: &[u8]) -> Result<(), ()> {
        assert!(offset.is_multiple_of(4) && bytes.len().is_multiple_of(4));
        for (i, word) in bytes.as_chunks::<4>().0.iter().enumerate() {
            if self.cut_after_words == Some(0) {
                return Err(());
            }
            if let Some(left) = &mut self.cut_after_words {
                *left -= 1;
            }
            for (j, value) in word.iter().enumerate() {
                let old = &mut self.pages[index(slot)][offset + i * 4 + j];
                assert_eq!(*old & *value, *value);
                *old = *value;
            }
            self.mutations += 1;
        }
        Ok(())
    }
}
fn key(seed: u8) -> [u8; 32] {
    tucket::identity::LocalIdentity::from_seed([seed; 32])
        .identity()
        .pub_key
}
fn scope() -> Scope {
    Scope::new(
        key(3),
        selvage::PhyProfile::meshcore(906875000, 250000, 10, 5),
    )
}
fn snapshot(keys: &[[u8; 32]]) -> Snapshot {
    Snapshot::new(scope(), keys).unwrap()
}

#[cfg(feature = "tucket-contact-store-bench")]
#[test]
fn fixture_handoff_refuses_other_keys_scopes_sequences_and_corruption() {
    let mut flash = Flash::default();
    assert!(fixture::clear_fixture(&mut flash).is_err());
    assert_eq!(flash.mutations, 0);
    fixture::run(&mut flash).unwrap();
    let good = flash.clone();
    let before = flash.mutations;
    fixture::clear_fixture(&mut flash).unwrap();
    assert_eq!(flash.mutations, before + 2);
    assert!(flash.pages.iter().flatten().all(|b| *b == 255));
    let expected = fixture::snapshot().unwrap();
    for scenario in 0..4 {
        let mut flash = good.clone();
        match scenario {
            0 => {
                let mut s = expected.clone();
                s.keys.pop();
                save(&mut flash, &s).unwrap();
            }
            1 => {
                let mut s = expected.clone();
                s.keys.clear();
                save(&mut flash, &s).unwrap();
                save(&mut flash, &expected).unwrap();
            }
            2 => {
                flash.pages[1][40] ^= 1;
            }
            _ => {
                flash = Flash::default();
                let mut s = expected.clone();
                s.scope.local_public = key(90);
                save(&mut flash, &s).unwrap();
            }
        }
        let before = flash.pages;
        let writes = flash.mutations;
        assert!(fixture::clear_fixture(&mut flash).is_err());
        assert_eq!(flash.pages, before);
        assert_eq!(flash.mutations, writes);
    }
}

#[test]
fn blank_save_cold_reload_and_repeated_adverts_are_bounded() {
    let mut flash = Flash::default();
    assert!(load(&mut flash, scope()).unwrap().is_none());
    let first = snapshot(&[key(4)]);
    assert_eq!(
        save(&mut flash, &first).unwrap(),
        Commit {
            slot: Slot::A,
            sequence: 0,
            changed: true
        }
    );
    let writes = flash.mutations;
    assert!(!save(&mut flash, &first).unwrap().changed);
    assert_eq!(flash.mutations, writes);
    let mut cold = flash.clone();
    let read = load(&mut cold, scope()).unwrap().unwrap();
    assert_eq!(read.snapshot, first);
    let mut node =
        tucket::node::Node::new(tucket::identity::LocalIdentity::from_seed([3; 32]), false);
    node.restore_contacts(&read.snapshot.keys).unwrap();
    assert_eq!(
        node.contact_keys().collect::<std::vec::Vec<_>>(),
        first.keys.as_slice()
    );
    assert!(node.route_to(key(4)[0]).is_none());
    let second = snapshot(&[key(4), key(9)]);
    assert_eq!(
        save(&mut cold, &second).unwrap(),
        Commit {
            slot: Slot::B,
            sequence: 1,
            changed: true
        }
    );
    assert_eq!(load(&mut cold, scope()).unwrap().unwrap().snapshot, second);
}

#[test]
fn every_interrupted_program_word_preserves_the_previous_committed_snapshot() {
    let mut good = Flash::default();
    let first = snapshot(&[key(4)]);
    save(&mut good, &first).unwrap();
    let active = good.pages[0];
    let second = snapshot(&[key(4), key(9)]);
    // All words, including the final commit magic. Erase affects only B.
    let words = store::encoded_len(72 + 64) / 4;
    for cut in 0..words {
        let mut fault = good.clone();
        fault.cut_after_words = Some(cut);
        assert_eq!(save(&mut fault, &second), Err(Fault::Io));
        assert_eq!(fault.pages[0], active);
        fault.cut_after_words = None;
        assert_eq!(
            load(&mut fault, scope()).unwrap().unwrap().snapshot,
            first,
            "word {cut}"
        );
        assert!(save(&mut fault, &second).unwrap().changed);
        assert_eq!(load(&mut fault, scope()).unwrap().unwrap().snapshot, second);
    }
}

fn replace_body(page: &mut [u8; SLOT_LEN], change: impl FnOnce(&mut [u8])) {
    let record = store::decode(page).unwrap();
    let sequence = record.sequence;
    let mut body = record.body.to_vec();
    change(&mut body);
    store::encode(sequence, &body, page).unwrap();
}

#[test]
fn malformed_future_and_mismatched_scope_never_trigger_a_repair_write() {
    let mut flash = Flash::default();
    let first = snapshot(&[key(4)]);
    save(&mut flash, &first).unwrap();
    save(&mut flash, &snapshot(&[key(4), key(9)])).unwrap();
    let pristine = flash.clone();
    let mut other = scope();
    other.local_public = key(6);
    assert!(matches!(load(&mut flash, other), Err(Fault::Scope)));
    assert_eq!(
        save(&mut flash, &Snapshot::new(other, &[]).unwrap()),
        Err(Fault::Scope)
    );
    for change in [0, 1, 2, 3] {
        let mut faulty = pristine.clone();
        replace_body(&mut faulty.pages[1], |body| match change {
            0 => body[4] = 2, // committed future version, even with a valid older slot
            1 => body[6] = 9, // count exceeds qualified profile
            2 => body[72..104].fill(0),
            _ => {
                let key: [u8; 32] = body[72..104].try_into().unwrap();
                body[104..136].copy_from_slice(&key);
            }
        });
        let before = faulty.pages;
        assert!(save(&mut faulty, &first).is_err());
        assert_eq!(faulty.pages, before);
        assert_eq!(faulty.mutations, pristine.mutations);
    }
    let mut blank = Flash::default();
    blank.pages[0][0] = 0;
    assert!(matches!(load(&mut blank, scope()), Err(Fault::Corrupt)));
    assert_eq!(blank.mutations, 0);
}

#[test]
fn first_write_cuts_leave_no_committed_cache_and_can_retry() {
    let snapshot = snapshot(&[key(4)]);
    for cut in 0..store::encoded_len(72 + 32) / 4 {
        let mut flash = Flash {
            cut_after_words: Some(cut),
            ..Flash::default()
        };
        assert_eq!(save(&mut flash, &snapshot), Err(Fault::Io));
        flash.cut_after_words = None;
        assert!(load(&mut flash, scope()).unwrap().is_none());
        assert!(save(&mut flash, &snapshot).unwrap().changed);
        assert_eq!(
            load(&mut flash, scope()).unwrap().unwrap().snapshot,
            snapshot
        );
    }
}

#[test]
fn sequence_exhaustion_and_conflicting_equal_sequences_are_refused() {
    let mut flash = Flash::default();
    save(&mut flash, &snapshot(&[key(4)])).unwrap();
    let body = store::decode(&flash.pages[0]).unwrap().body.to_vec();
    store::encode(u32::MAX, &body, &mut flash.pages[0]).unwrap();
    let before = flash.pages;
    assert_eq!(
        save(&mut flash, &snapshot(&[key(9)])),
        Err(Fault::Exhausted)
    );
    assert_eq!(flash.pages, before);
    flash.pages[1] = flash.pages[0];
    replace_body(&mut flash.pages[1], |body| {
        body[72..104].copy_from_slice(&key(9))
    });
    assert!(matches!(load(&mut flash, scope()), Err(Fault::Ambiguous)));
}

#[test]
fn readback_failure_keeps_old_slot_and_reports_uncertain_new_commit() {
    let mut flash = Flash::default();
    save(&mut flash, &snapshot(&[key(4)])).unwrap();
    let old = flash.pages[0];
    flash.refuse_read = Some(flash.reads + 3);
    let second = snapshot(&[key(4), key(9)]);
    assert_eq!(save(&mut flash, &second), Err(Fault::Io));
    assert_eq!(flash.pages[0], old);
    flash.refuse_read = None;
    // Complete program happened before the read error. Never promise rollback.
    assert_eq!(load(&mut flash, scope()).unwrap().unwrap().snapshot, second);
}

#[test]
fn scope_covers_all_receive_parameters_and_excludes_power_only() {
    let profile = selvage::PhyProfile::meshcore(906875000, 250000, 10, 5);
    let original = Scope::new(key(3), profile);
    let mut power = profile;
    power.tx_power_dbm = 0;
    assert_eq!(Scope::new(key(3), power), original);
    let mut changed = profile;
    changed.invert_iq = !changed.invert_iq;
    assert_ne!(Scope::new(key(3), changed), original);
    changed = profile;
    changed.frequency_hz += 1000;
    assert_ne!(Scope::new(key(3), changed), original);
}
