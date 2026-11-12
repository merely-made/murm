//! The three SDP edits the relay path needs, and nothing else.
//!
//! `webrtc-carrier` hands out and takes in SDP *text*, which is the seam this
//! module works in. There is no SDP parser here and none is wanted: these are
//! line-level edits on `a=candidate:` lines, done in the probe because the
//! probe owns the offer it feeds to str0m and the answer it returns from
//! `/offer`.
//!
//! 1. [`peer_candidates`] — read the browser's candidate addresses out of the
//!    offer, so the TURN client knows who to permit and channel-bind.
//! 2. [`rewrite_candidates`] — point every one of them at the local shim
//!    socket standing in for that peer, so everything str0m sends to the peer
//!    is framed toward the relay instead of thrown at an address that would
//!    drop it.
//! 3. [`inject_relay_candidate`] — add the `typ relay` line to the answer.
//!
//! ## Why the relay candidate is injected rather than advertised
//!
//! `AnswererConfig::advertise` takes bare `IpAddr`s and
//! `Answerer::bind` turns each into `Candidate::host(SocketAddr::new(ip,
//! local_port), "udp")`. Two things about that shut the door on a relayed
//! candidate: the *port* is forced to the carrier's own bound port, and the
//! *type* is forced to host. A relay allocation is a different port on a
//! different host and must be offered as `typ relay` — a browser under
//! `iceTransportPolicy: "relay"` discards every remote candidate that is not.
//! There is no accessor on `Answerer` for the underlying `Rtc` either, so
//! `add_local_candidate` with `Candidate::relayed(..)` is not reachable from
//! outside the crate.
//!
//! Carrying a relay candidate through `advertise` is therefore a
//! `webrtc-carrier` change, and that crate is frozen. Injecting the line into
//! the answer text is the honest alternative, and it costs nothing str0m
//! needs: str0m is not asked to relay anything, because the shim rewrite in
//! (2) means every packet it sends to the peer already goes to a socket this
//! process owns.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

/// The priority ICE gives a relayed candidate: type preference 0, the highest
/// local preference, component 1.
///
/// `(0 << 24) | (65535 << 8) | (256 - 1)`.
const RELAY_PRIORITY: u32 = (65535 << 8) | 255;

/// Splits SDP into lines that remember their own terminator, so a rewrite of
/// a CRLF document stays a CRLF document.
fn lines(sdp: &str) -> impl Iterator<Item = (&str, &str)> {
    sdp.split_inclusive('\n').map(|raw| {
        let body = raw.trim_end_matches('\n').trim_end_matches('\r');
        (body, &raw[body.len()..])
    })
}

/// One `a=candidate:` line, split far enough to rewrite.
struct CandidateLine<'a> {
    foundation: &'a str,
    component: &'a str,
    transport: &'a str,
    priority: &'a str,
    addr: SocketAddr,
    kind: &'a str,
    /// Everything after `typ <kind>`, as the name/value pairs SDP defines it
    /// to be.
    extensions: Vec<(&'a str, &'a str)>,
}

impl<'a> CandidateLine<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let rest = line.strip_prefix("a=candidate:")?;
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() < 8 || !tokens[6].eq_ignore_ascii_case("typ") {
            return None;
        }
        let ip: IpAddr = tokens[4].parse().ok()?;
        let port: u16 = tokens[5].parse().ok()?;
        let extensions = tokens[8..]
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| (pair[0], pair[1]))
            .collect();
        Some(Self {
            foundation: tokens[0],
            component: tokens[1],
            transport: tokens[2],
            priority: tokens[3],
            addr: SocketAddr::new(ip, port),
            kind: tokens[7],
            extensions,
        })
    }

    fn is_udp(&self) -> bool {
        self.transport.eq_ignore_ascii_case("udp")
    }
}

/// Every distinct UDP candidate address the SDP offers, in wire order.
///
/// Candidates whose connection-address is not a literal IP are skipped —
/// Chrome's default host candidates are `<uuid>.local` mDNS names, which
/// nothing here can resolve and which never appear under a relay policy
/// anyway — and so is anything but UDP, since this client allocates
/// `REQUESTED-TRANSPORT: UDP`.
pub fn peer_candidates(sdp: &str) -> Vec<SocketAddr> {
    let mut seen = Vec::new();
    for (line, _) in lines(sdp) {
        let Some(candidate) = CandidateLine::parse(line) else {
            continue;
        };
        if candidate.is_udp() && !seen.contains(&candidate.addr) {
            seen.push(candidate.addr);
        }
    }
    seen
}

/// Rewrites each candidate to the local shim standing in for that peer.
///
/// The rewritten line is `typ host` at the shim's address: from str0m's side
/// the peer simply lives on this machine. `raddr`/`rport` are dropped, because
/// they are only meaningful for a derived candidate type and a host candidate
/// carrying them is malformed; every other extension (`generation`, `ufrag`,
/// `network-id`, …) rides along untouched.
///
/// A candidate with no mapping is dropped rather than left pointing at an
/// address the relay path cannot reach. If that would empty the offer, the
/// original is returned unchanged — an offer with no candidates at all is a
/// worse failure than an offer str0m cannot use, and the caller's fallback
/// reads better than a silent hang.
pub fn rewrite_candidates(sdp: &str, shims: &HashMap<SocketAddr, SocketAddr>) -> String {
    let mut out = String::with_capacity(sdp.len() + 64);
    let mut kept = 0usize;
    let mut had = 0usize;
    for (line, terminator) in lines(sdp) {
        // Anything that is not a candidate travels verbatim.
        if !line.starts_with("a=candidate:") {
            out.push_str(line);
            out.push_str(terminator);
            continue;
        }
        had += 1;
        // A candidate this module cannot re-point — an mDNS name it cannot
        // resolve, a TCP candidate the allocation cannot carry, a shape it
        // does not recognise — is dropped rather than left aimed somewhere the
        // relay path will never deliver from.
        let Some(candidate) = CandidateLine::parse(line) else {
            continue;
        };
        let Some(shim) = shims.get(&candidate.addr) else {
            continue;
        };
        kept += 1;
        out.push_str("a=candidate:");
        out.push_str(candidate.foundation);
        out.push(' ');
        out.push_str(candidate.component);
        out.push(' ');
        out.push_str(candidate.transport);
        out.push(' ');
        out.push_str(candidate.priority);
        out.push_str(&format!(" {} {} typ host", shim.ip(), shim.port()));
        for (name, value) in &candidate.extensions {
            if name.eq_ignore_ascii_case("raddr") || name.eq_ignore_ascii_case("rport") {
                continue;
            }
            out.push_str(&format!(" {name} {value}"));
        }
        out.push_str(terminator);
    }
    if had > 0 && kept == 0 {
        return sdp.to_owned();
    }
    out
}

/// Adds a `typ relay` candidate for `relayed` to an answer.
///
/// `base` is this client's own server-reflexive address as the TURN server
/// reported it, which is what `raddr`/`rport` mean for a relayed candidate;
/// absent one, the unspecified address is used, which every stack accepts.
/// The candidate is placed immediately after the last existing `a=candidate:`
/// line so it sits inside the media section it belongs to, and carries the
/// answer's own `ice-ufrag` when there is one, matching how str0m writes its
/// own candidate lines.
pub fn inject_relay_candidate(
    answer: &str,
    relayed: SocketAddr,
    base: Option<SocketAddr>,
) -> String {
    let ufrag = lines(answer)
        .filter_map(|(line, _)| line.strip_prefix("a=ice-ufrag:"))
        .next();
    let (raddr, rport) = match base {
        Some(base) => (base.ip().to_string(), base.port()),
        None if relayed.is_ipv6() => ("::".to_owned(), 0),
        None => ("0.0.0.0".to_owned(), 0),
    };
    // A foundation only has to be stable for candidates of the same type,
    // protocol and base; one derived from the relayed address is both.
    let foundation = {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in relayed.to_string().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{:012x}", hash & 0xffff_ffff_ffff)
    };
    let mut line = format!(
        "a=candidate:{foundation} 1 udp {RELAY_PRIORITY} {} {} typ relay raddr {raddr} rport {rport}",
        relayed.ip(),
        relayed.port(),
    );
    if let Some(ufrag) = ufrag {
        line.push_str(&format!(" ufrag {ufrag}"));
    }

    let terminator = if answer.contains("\r\n") { "\r\n" } else { "\n" };
    let anchor = lines(answer)
        .enumerate()
        .filter(|(_, (body, _))| body.starts_with("a=candidate:"))
        .map(|(index, _)| index)
        .last()
        .or_else(|| {
            lines(answer)
                .position(|(body, _)| body.starts_with("a=ice-ufrag:"))
                .map(|index| index.saturating_sub(1))
        });

    let mut out = String::with_capacity(answer.len() + line.len() + 2);
    let mut inserted = false;
    for (index, (body, own_terminator)) in lines(answer).enumerate() {
        out.push_str(body);
        out.push_str(if own_terminator.is_empty() {
            terminator
        } else {
            own_terminator
        });
        if Some(index) == anchor {
            out.push_str(&line);
            out.push_str(terminator);
            inserted = true;
        }
    }
    if !inserted {
        out.push_str(&line);
        out.push_str(terminator);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELAY_OFFER: &str = "v=0\r\n\
o=- 1 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=candidate:1 1 udp 41885439 192.168.4.28 51111 typ relay raddr 0.0.0.0 rport 0 generation 0\r\n\
a=candidate:2 1 tcp 1518214911 192.168.4.28 51112 typ relay tcptype passive raddr 0.0.0.0 rport 0\r\n\
a=candidate:3 1 udp 2122260223 abcd-1234.local 55000 typ host generation 0\r\n\
a=ice-ufrag:probe\r\n\
a=ice-pwd:probeprobeprobeprobepr\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n";

    #[test]
    fn peer_candidates_takes_udp_literals_only() {
        // The TCP relay candidate and the mDNS host name are both unusable by
        // a UDP allocation, and permitting an address we cannot reach would
        // spend a CreatePermission on nothing.
        assert_eq!(
            peer_candidates(RELAY_OFFER),
            vec!["192.168.4.28:51111".parse::<SocketAddr>().unwrap()]
        );
        assert!(peer_candidates("v=0\r\n").is_empty());
    }

    #[test]
    fn rewrite_points_candidates_at_the_shim_as_host() {
        let mut shims = HashMap::new();
        shims.insert(
            "192.168.4.28:51111".parse().unwrap(),
            "192.168.4.36:60001".parse::<SocketAddr>().unwrap(),
        );
        let rewritten = rewrite_candidates(RELAY_OFFER, &shims);

        assert!(rewritten.contains(
            "a=candidate:1 1 udp 41885439 192.168.4.36 60001 typ host generation 0\r\n"
        ));
        // raddr/rport are meaningless on a host candidate and are gone.
        assert!(!rewritten.contains("raddr"));
        assert!(!rewritten.contains("rport"));
        // Unmapped candidates are dropped, not left aimed at the relay.
        assert!(!rewritten.contains("51112"));
        assert!(!rewritten.contains(".local"));
        // Everything that is not a candidate survives verbatim, CRLF included.
        assert!(rewritten.starts_with("v=0\r\n"));
        assert!(rewritten.contains("a=ice-ufrag:probe\r\n"));
        assert!(rewritten.ends_with("a=mid:0\r\n"));
    }

    #[test]
    fn rewrite_keeps_the_original_rather_than_emptying_it() {
        let rewritten = rewrite_candidates(RELAY_OFFER, &HashMap::new());
        assert_eq!(rewritten, RELAY_OFFER);
        // An offer that never had candidates is passed through either way.
        assert_eq!(rewrite_candidates("v=0\r\n", &HashMap::new()), "v=0\r\n");
    }

    /// The acceptance bar for C3's forced-relay receipt, asserted on the exact
    /// shape str0m 0.23 writes: without a `typ relay` line in the answer,
    /// Chrome under `iceTransportPolicy: "relay"` has nothing to pair with.
    #[test]
    fn the_answer_gains_a_typ_relay_candidate_after_the_host_ones() {
        let answer = "v=0\r\n\
o=str0m-0.23.1 1 2 IN IP4 0.0.0.0\r\n\
s=-\r\n\
t=0 0\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=candidate:fffeff7e 1 udp 2130706175 192.168.4.36 58512 typ host ufrag VDKJV1q4dc1UNKLe\r\n\
a=candidate:fffcff7e 1 udp 2130705663 127.0.0.1 58512 typ host ufrag VDKJV1q4dc1UNKLe\r\n\
a=ice-ufrag:VDKJV1q4dc1UNKLe\r\n\
a=ice-pwd:NsOdX5XQ3ie0BHEPBKzICH\r\n\
a=setup:passive\r\n\
a=mid:0\r\n";
        let out = inject_relay_candidate(
            answer,
            "192.168.4.28:49185".parse().unwrap(),
            Some("192.168.4.36:58600".parse().unwrap()),
        );

        let relay = out
            .lines()
            .find(|line| line.contains("typ relay"))
            .expect("the answer must offer a relay candidate");
        let tokens: Vec<&str> = relay.split(' ').collect();
        assert!(tokens[0].starts_with("a=candidate:"), "{relay}");
        assert_eq!(
            &tokens[1..],
            &[
                "1",
                "udp",
                "16777215",
                "192.168.4.28",
                "49185",
                "typ",
                "relay",
                "raddr",
                "192.168.4.36",
                "rport",
                "58600",
                "ufrag",
                "VDKJV1q4dc1UNKLe",
            ],
            "unexpected relay candidate line: {relay}"
        );
        // The injected line must itself parse as a candidate, or a browser
        // will drop the one candidate the whole receipt depends on.
        let parsed = CandidateLine::parse(relay).expect("the injected line must parse");
        assert_eq!(parsed.kind, "relay");
        assert_eq!(
            parsed.addr,
            "192.168.4.28:49185".parse::<SocketAddr>().unwrap()
        );
        // Placed with the other candidates, before the ice-ufrag line, and the
        // rest of the answer is untouched.
        let relay_at = out.find("typ relay").unwrap();
        assert!(relay_at > out.find("127.0.0.1 58512").unwrap());
        assert!(relay_at < out.find("a=ice-ufrag:").unwrap());
        assert!(out.starts_with("v=0\r\n") && out.ends_with("a=mid:0\r\n"));
        assert!(!out.contains("\n\r\n"), "no blank line was introduced");
        // Deterministic: a redeploy of the same allocation writes the same line.
        assert_eq!(
            out,
            inject_relay_candidate(
                answer,
                "192.168.4.28:49185".parse().unwrap(),
                Some("192.168.4.36:58600".parse().unwrap())
            )
        );
    }

    #[test]
    fn injection_survives_an_answer_with_no_candidates() {
        let bare = "v=0\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\na=ice-ufrag:zz\na=mid:0\n";
        let out = inject_relay_candidate(bare, "192.168.4.28:49185".parse().unwrap(), None);
        assert!(out.contains("typ relay raddr 0.0.0.0 rport 0 ufrag zz"));
        assert!(out.find("typ relay").unwrap() < out.find("a=ice-ufrag:").unwrap());
        assert!(!out.contains("\r\n"), "an LF answer stays an LF answer");
    }

    #[test]
    fn the_relay_priority_is_the_one_ice_specifies() {
        // type preference 0, local preference 65535, component 1.
        assert_eq!(RELAY_PRIORITY, 16_777_215);
    }
}
