//! Per-interface path-request frequencies: ingress bursts and egress control
//! (`Interface.py` 213-250, 315-325, 368-389).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::announce_admission::AnnounceIngressPolicy;

use super::super::iface_policy::PathRequestPolicy;

/// Timestamps kept per direction (`IP_FREQ_SAMPLES`, `OP_FREQ_SAMPLES`).
const SAMPLES: usize = 48;
/// The oldest sample is dropped once it is older than this (`PR_FREQ_DECAY`).
const DECAY: Duration = Duration::from_secs(10);
/// Incoming samples needed before a frequency is reported (`IC_DEQUE_MIN_SAMPLE`).
const INGRESS_MIN_SAMPLES: usize = 2;
/// Outgoing samples needed before egress control acts (`EC_BURST_MIN_SAMPLES`).
const EGRESS_MIN_SAMPLES: usize = 2;
/// Calm evaluations a burst sits out before it ends (`ic_pr_burst_cooldown`).
const COOLDOWN: u8 = 3;

#[derive(Debug, Clone, Copy)]
struct Burst {
    activated: Instant,
    sustained: Instant,
    cooldown: u8,
}

/// One interface's path-request history.
#[derive(Debug)]
pub(super) struct PrRates {
    /// When this node first heard a path request here: the interface's age for the burst
    /// threshold (RNS counts from creation).
    first_seen: Instant,
    incoming: VecDeque<Instant>,
    outgoing: VecDeque<Instant>,
    burst: Option<Burst>,
}

/// `n` samples over the span since the oldest, which is dropped once older than [`DECAY`];
/// zero below `min` samples or over no time.
fn frequency(samples: &mut VecDeque<Instant>, n: usize, min: usize, now: Instant) -> f64 {
    if samples.len() <= min {
        return 0.0;
    }
    let span = now.saturating_duration_since(samples[0]);
    if span > DECAY {
        samples.pop_front();
    }
    if span.is_zero() {
        return 0.0;
    }
    n as f64 / span.as_secs_f64()
}

fn record(samples: &mut VecDeque<Instant>, now: Instant) {
    if samples.len() == SAMPLES {
        samples.pop_front();
    }
    samples.push_back(now);
}

impl PrRates {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            first_seen: now,
            incoming: VecDeque::new(),
            outgoing: VecDeque::new(),
            burst: None,
        }
    }

    /// Count a path request heard here (`received_path_request`).
    pub(super) fn received(&mut self, now: Instant) {
        record(&mut self.incoming, now);
    }

    /// Count a path request sent here (`sent_path_request`).
    pub(super) fn sent(&mut self, now: Instant) {
        record(&mut self.outgoing, now);
    }

    /// Whether this interface is in a path-request burst, updating it (`should_ingress_limit_pr`).
    /// A burst starts above the threshold, which is the stricter one while the interface is
    /// new; it ends after `burst_hold` below it and [`COOLDOWN`] calm evaluations more.
    pub(super) fn ingress_limited(
        &mut self,
        now: Instant,
        rates: &PathRequestPolicy,
        ingress: &AnnounceIngressPolicy,
    ) -> bool {
        if !ingress.enabled {
            return false;
        }
        let new = now.saturating_duration_since(self.first_seen) < ingress.new_interface_age;
        let threshold = if new {
            rates.burst_hz_new
        } else {
            rates.burst_hz
        } as f64;
        let n = self.incoming.len();
        let hz = frequency(&mut self.incoming, n, INGRESS_MIN_SAMPLES, now);
        let hold = ingress.burst_hold;
        match &mut self.burst {
            Some(burst) => {
                if hz < threshold && now > burst.activated + hold && now > burst.sustained + hold {
                    if burst.cooldown == 0 {
                        self.burst = None;
                    } else {
                        burst.cooldown -= 1;
                    }
                } else {
                    burst.cooldown = COOLDOWN;
                    if hz >= threshold {
                        burst.sustained = now;
                    }
                }
                true
            }
            None if hz > threshold => {
                self.burst = Some(Burst {
                    activated: now,
                    sustained: now,
                    cooldown: COOLDOWN,
                });
                true
            }
            None => false,
        }
    }

    /// Whether egress control withholds one more path request here, counting it in advance
    /// (`should_egress_limit_pr`, `outgoing_pr_frequency(preemptive=True)`).
    pub(super) fn egress_limited(&mut self, now: Instant, rates: &PathRequestPolicy) -> bool {
        if !rates.egress_control {
            return false;
        }
        let n = self.outgoing.len() + 1;
        let hz = frequency(&mut self.outgoing, n, 1, now);
        hz > rates.egress_hz as f64 && self.outgoing.len() >= EGRESS_MIN_SAMPLES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(at: Instant, ms: u64) -> Instant {
        at + Duration::from_millis(ms)
    }

    #[test]
    fn a_burst_starts_above_the_new_interface_rate_and_cools_down() {
        let (rates, ingress) = (
            PathRequestPolicy::default(),
            AnnounceIngressPolicy::default(),
        );
        let t0 = Instant::now();
        let mut pr = PrRates::new(t0);
        for i in 0..2 {
            pr.received(ms(t0, i * 100));
            assert!(
                !pr.ingress_limited(ms(t0, i * 100), &rates, &ingress),
                "too few"
            );
        }
        pr.received(ms(t0, 200));
        assert!(
            pr.ingress_limited(ms(t0, 200), &rates, &ingress),
            "3 in 0.2 s"
        );
        // Quiet past the hold: still limited through the cooldown, then not.
        let late = ms(t0, 60_000);
        for _ in 0..=COOLDOWN {
            assert!(pr.ingress_limited(late, &rates, &ingress));
        }
        assert!(!pr.ingress_limited(late, &rates, &ingress));

        let off = AnnounceIngressPolicy {
            enabled: false,
            ..ingress
        };
        let mut pr = PrRates::new(t0);
        (0..10).for_each(|i| pr.received(ms(t0, i)));
        assert!(!pr.ingress_limited(ms(t0, 10), &rates, &off));
    }

    #[test]
    fn egress_control_counts_the_request_about_to_go() {
        let rates = PathRequestPolicy {
            egress_control: true,
            egress_hz: 5,
            ..PathRequestPolicy::default()
        };
        let t0 = Instant::now();
        let mut pr = PrRates::new(t0);
        assert!(!pr.egress_limited(t0, &rates));
        pr.sent(t0);
        pr.sent(ms(t0, 200));
        // Three over 0.5 s is 6 Hz.
        assert!(pr.egress_limited(ms(t0, 500), &rates));
        assert!(!pr.egress_limited(ms(t0, 700), &rates), "three over 0.7 s");
        assert!(!pr.egress_limited(t0, &PathRequestPolicy::default()));
    }
}
