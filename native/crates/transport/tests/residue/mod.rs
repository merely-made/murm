// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The no-residue tracker (vault lock plan, ruling 6), shared by personae's
//! instrument and castellan's (L2), which includes this file by path.
//!
//! A tracking global allocator owns the binary's heap. Canary keys are
//! planted; while armed, every freed block is scanned before it reaches the
//! system allocator, and at disarm every block still live is scanned too. A
//! hit is a canary (any 16-byte window of it, raw or as serde_json's decimal
//! array) left in memory nobody owns. Stack copies and memory the OS
//! allocates are outside what this can see.

// Each includer uses its own subset.
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ─── The tracker ──────────────────────────────────────────────────────────

const WINDOW: usize = 16;
const MAX_CANARY: usize = 64;
const MAX_CANARIES: usize = 4;
const TABLE: usize = 1 << 20;
const TOMBSTONE: usize = usize::MAX;
const MAX_HITS: usize = 64;

struct Canary {
    bytes: [u8; MAX_CANARY],
    len: usize,
    /// Byte positions by value (raw match anchors).
    at: [[u8; 4]; 256],
    at_count: [u8; 256],
    /// Unique position per value when the canary's bytes are distinct
    /// (needed for the decimal-array match), else -1.
    unique: [i16; 256],
    json: bool,
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub phase: usize,
    pub live: bool,
    pub canary: usize,
    pub json: bool,
    pub size: usize,
    /// The phase the block was allocated in; `UNTRACKED` if before arming.
    pub born: usize,
}

pub const UNTRACKED: usize = usize::MAX;

struct State {
    canaries: [Option<Canary>; MAX_CANARIES],
    hits: [Option<Hit>; MAX_HITS],
    hit_count: usize,
    live_ptr: [usize; TABLE],
    live_size: [usize; TABLE],
    live_born: [usize; TABLE],
    tracked: usize,
    overflow: bool,
}

struct Shared(UnsafeCell<State>);
// Every access holds `LOCK`.
unsafe impl Sync for Shared {}

static STATE: Shared = Shared(UnsafeCell::new(State {
    canaries: [const { None }; MAX_CANARIES],
    hits: [None; MAX_HITS],
    hit_count: 0,
    live_ptr: [0; TABLE],
    live_size: [0; TABLE],
    live_born: [0; TABLE],
    tracked: 0,
    overflow: false,
}));
static LOCK: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static PHASE: AtomicUsize = AtomicUsize::new(0);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
    }
    let out = f(unsafe { &mut *STATE.0.get() });
    LOCK.store(false, Ordering::Release);
    out
}

fn slot_of(ptr: usize) -> usize {
    (ptr >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) % TABLE
}

impl State {
    fn insert(&mut self, ptr: usize, size: usize) {
        let mut i = slot_of(ptr);
        for _ in 0..TABLE {
            if self.live_ptr[i] == 0 || self.live_ptr[i] == TOMBSTONE {
                self.live_ptr[i] = ptr;
                self.live_size[i] = size;
                self.live_born[i] = PHASE.load(Ordering::Relaxed);
                self.tracked += 1;
                return;
            }
            i = (i + 1) % TABLE;
        }
        self.overflow = true;
    }

    /// Forget a block; returns the phase it was born in.
    fn remove(&mut self, ptr: usize) -> usize {
        if self.tracked == 0 {
            return UNTRACKED;
        }
        let mut i = slot_of(ptr);
        for _ in 0..TABLE {
            match self.live_ptr[i] {
                0 => return UNTRACKED,
                p if p == ptr => {
                    self.live_ptr[i] = TOMBSTONE;
                    self.tracked -= 1;
                    return self.live_born[i];
                },
                _ => i = (i + 1) % TABLE,
            }
        }
        UNTRACKED
    }

    fn scan(&mut self, block: &[u8], live: bool, born: usize) {
        for c in 0..MAX_CANARIES {
            let Some(canary) = &self.canaries[c] else {
                continue;
            };
            let raw = raw_hit(canary, block);
            let json = !raw && canary.json && json_hit(canary, block);
            if (raw || json) && self.hit_count < MAX_HITS {
                self.hits[self.hit_count] = Some(Hit {
                    phase: PHASE.load(Ordering::Relaxed),
                    live,
                    canary: c,
                    json,
                    size: block.len(),
                    born,
                });
                self.hit_count += 1;
            } else if raw || json {
                self.hit_count = MAX_HITS;
            }
        }
    }
}

fn raw_hit(c: &Canary, block: &[u8]) -> bool {
    if block.len() < WINDOW {
        return false;
    }
    for i in 0..=block.len() - WINDOW {
        let b = block[i] as usize;
        for k in 0..c.at_count[b] as usize {
            let j = c.at[b][k] as usize;
            if j + WINDOW <= c.len && block[i..i + WINDOW] == c.bytes[j..j + WINDOW] {
                return true;
            }
        }
    }
    false
}

/// Any 16 consecutive canary bytes written as `a,b,c,...` decimals.
fn json_hit(c: &Canary, block: &[u8]) -> bool {
    let mut run = 0usize;
    let mut expect: i32 = -1;
    let mut num: u32 = 0;
    let mut in_num = false;
    let finish = |num: u32, run: &mut usize, expect: &mut i32| -> bool {
        let j = if num <= 255 {
            c.unique[num as usize] as i32
        } else {
            -1
        };
        if j < 0 {
            *run = 0;
            *expect = -1;
            return false;
        }
        *run = if *run > 0 && j == *expect {
            *run + 1
        } else {
            1
        };
        *expect = j + 1;
        *run >= WINDOW
    };
    for &ch in block {
        if ch.is_ascii_digit() {
            num = (num * 10 + u32::from(ch - b'0')).min(1000);
            in_num = true;
            continue;
        }
        if in_num && finish(num, &mut run, &mut expect) {
            return true;
        }
        in_num = false;
        num = 0;
        if ch != b',' {
            run = 0;
        }
    }
    in_num && finish(num, &mut run, &mut expect)
}

struct Tracker;

unsafe impl GlobalAlloc for Tracker {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && ARMED.load(Ordering::Relaxed) {
            // Fresh blocks start zeroed while armed, so a hit is never stale
            // heap left from before arming.
            unsafe { std::ptr::write_bytes(ptr, 0, layout.size()) };
            with_state(|s| s.insert(ptr as usize, layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let armed = ARMED.load(Ordering::Relaxed);
        with_state(|s| {
            let born = s.remove(ptr as usize);
            if armed {
                let block = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
                s.scan(block, false, born);
            }
        });
        unsafe { System.dealloc(ptr, layout) }
    }
    // `realloc` keeps the default: alloc, copy, dealloc, so the old block is
    // scanned like any other free.
}

#[global_allocator]
static GLOBAL: Tracker = Tracker;

// ─── Harness ──────────────────────────────────────────────────────────────

pub fn plant(index: usize, label: &'static str, bytes: &[u8], json: bool) {
    LABELS.lock().unwrap()[index] = label;
    assert!(bytes.len() <= MAX_CANARY && bytes.len() >= WINDOW);
    let mut canary = Canary {
        bytes: [0; MAX_CANARY],
        len: bytes.len(),
        at: [[0; 4]; 256],
        at_count: [0; 256],
        unique: [-1; 256],
        json,
    };
    canary.bytes[..bytes.len()].copy_from_slice(bytes);
    for (j, &b) in bytes.iter().enumerate() {
        // A fifth repeat of one byte value only loses that anchor.
        let n = &mut canary.at_count[b as usize];
        if *n < 4 {
            canary.at[b as usize][*n as usize] = j as u8;
            *n += 1;
        }
        if json {
            assert_eq!(
                canary.unique[b as usize], -1,
                "decimal canary needs distinct bytes"
            );
            canary.unique[b as usize] = j as i16;
        }
    }
    with_state(|s| s.canaries[index] = Some(canary));
}

/// Stop looking for canary `index`.
pub fn forget_canary(index: usize) {
    with_state(|s| s.canaries[index] = None);
}

pub fn clear_canaries() {
    with_state(|s| s.canaries = [const { None }; MAX_CANARIES]);
}

pub fn arm() {
    with_state(|s| {
        s.hits = [None; MAX_HITS];
        s.hit_count = 0;
    });
    ARMED.store(true, Ordering::SeqCst);
}

/// Scan what is still live, disarm, and return every hit.
pub fn disarm() -> (Vec<Hit>, bool) {
    PHASE.store(99, Ordering::SeqCst);
    let (hits, overflow) = with_state(|s| {
        for i in 0..TABLE {
            let p = s.live_ptr[i];
            if p != 0 && p != TOMBSTONE {
                let block = unsafe { std::slice::from_raw_parts(p as *const u8, s.live_size[i]) };
                let born = s.live_born[i];
                s.scan(block, true, born);
            }
        }
        let mut out = [None; MAX_HITS];
        out[..s.hit_count.min(MAX_HITS)].copy_from_slice(&s.hits[..s.hit_count.min(MAX_HITS)]);
        let overflow = s.overflow;
        s.live_ptr = [0; TABLE];
        s.tracked = 0;
        (out, overflow)
    });
    ARMED.store(false, Ordering::SeqCst);
    (hits.into_iter().flatten().collect(), overflow)
}

pub fn phase(n: usize) {
    PHASE.store(n, Ordering::SeqCst);
}

/// Distinct bytes with no visible pattern: an odd-step walk of 0..=255.
pub fn canary_bytes<const N: usize>(offset: usize) -> [u8; N] {
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = ((offset + i) * 167 + 59) as u8;
    }
    out
}

/// What each canary slot holds in the current scenario, for the report.
pub static LABELS: std::sync::Mutex<[&str; MAX_CANARIES]> =
    std::sync::Mutex::new(["-"; MAX_CANARIES]);

pub struct Report {
    pub failures: usize,
}

impl Report {
    pub fn check(&mut self, scenario: &str, phases: &[&str], hits: &[Hit], overflow: bool) {
        assert!(!overflow, "{scenario}: allocation table overflowed");
        if hits.is_empty() {
            println!("residue {scenario}: clean");
            return;
        }
        self.failures += 1;
        println!("residue {scenario}: {} hit(s)", hits.len());
        let name = |p: usize| match p {
            99 => "end of scenario",
            UNTRACKED => "before arming",
            p => phases.get(p).copied().unwrap_or("?"),
        };
        for h in hits {
            println!(
                "  {} {} ({}) in a {}-byte block, phase {} (allocated in {})",
                if h.live { "live" } else { "freed uncleared" },
                LABELS.lock().unwrap()[h.canary],
                if h.json { "decimal JSON" } else { "raw" },
                h.size,
                name(h.phase),
                name(h.born),
            );
        }
    }
}

/// The instrument proves itself in the same process before any scenario:
/// an uncleared Vec, raw and as decimal JSON, must be found, a `Zeroizing`
/// one must not, and a live one must be. Panics when it fails.
pub fn positive_control(seed: &[u8; 32]) {
    plant(0, "master seed", seed, true);
    arm();
    phase(1);
    drop(std::hint::black_box(seed.to_vec()));
    phase(2);
    drop(std::hint::black_box(serde_json::to_vec(seed).unwrap()));
    phase(3);
    drop(std::hint::black_box(zeroize::Zeroizing::new(seed.to_vec())));
    phase(4);
    let kept = std::hint::black_box(seed.to_vec());
    let (hits, _) = disarm();
    drop(kept);
    let saw = |p: usize, live: bool, json: bool| {
        hits.iter()
            .any(|h| h.phase == p && h.live == live && h.json == json)
    };
    let found = |yes: bool| if yes { "found" } else { "MISSED" };
    let zeroized_flagged = hits.iter().any(|h| h.phase == 3);
    println!(
        "positive control: uncleared raw {} / uncleared decimal {} / zeroized {} / live {}",
        found(saw(1, false, false)),
        found(saw(2, false, true)),
        if zeroized_flagged { "FLAGGED" } else { "clean" },
        found(saw(99, true, false)),
    );
    assert!(
        saw(1, false, false) && saw(2, false, true) && !zeroized_flagged && saw(99, true, false),
        "the instrument failed its positive control"
    );
}
