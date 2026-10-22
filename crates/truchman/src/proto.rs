//! The sans-io core: commands become validated lines, and bytes from a bridge become reply
//! lines plus whatever followed them. The `tokio` shell only moves bytes through these.
//!
//! Every value written onto a line is checked, so a configured name, key or option can never
//! smuggle a second command onto it.

use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::dest::{Destination, PrivateKey, SIGNATURE_ED25519};
use crate::reply::Reply;
use crate::{Error, Result};

/// The longest reply line accepted. A private key with a large certificate is about 1 KiB of
/// base64; this leaves room for long `MESSAGE` values without letting a bridge grow it.
pub const MAX_LINE: usize = 16 * 1024;

/// Which destination a session speaks for.
#[derive(Clone, Debug)]
pub enum Key {
    /// A fresh destination the router generates and forgets when the session ends.
    Transient,
    /// A stored destination, so the session is reachable at the same address every time.
    Persistent(PrivateKey),
}

/// How a session is created.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// The session ID; `None` picks a unique one.
    pub nickname: Option<String>,
    /// Signature type of a transient destination; ignored for a persistent one.
    pub signature_type: u16,
    /// I2CP and SAM options, written as `key=value` after the session's own (tunnel lengths,
    /// `i2cp.leaseSetEncType`, `i2cp.dontPublishLeaseSet`, ...). Empty leaves every one at the
    /// router's default, as RNS does.
    pub options: Vec<(String, String)>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            nickname: None,
            signature_type: SIGNATURE_ED25519,
            options: Vec::new(),
        }
    }
}

/// One command a client sends.
#[derive(Clone, Copy, Debug)]
pub enum Command<'a> {
    /// `HELLO VERSION MIN=3.1 MAX=3.3`, the first line on every connection.
    Hello,
    /// `DEST GENERATE`.
    DestGenerate {
        /// Signature type of the new destination.
        signature_type: u16,
    },
    /// `NAMING LOOKUP`.
    NamingLookup {
        /// A host name or `.b32.i2p` name.
        name: &'a str,
    },
    /// `SESSION CREATE STYLE=STREAM`.
    SessionCreate {
        /// Session ID.
        id: &'a str,
        /// The destination to speak for.
        key: &'a Key,
        /// Signature type and options.
        options: &'a SessionOptions,
    },
    /// `STREAM CONNECT`, unsilenced.
    StreamConnect {
        /// Session ID.
        id: &'a str,
        /// The remote destination.
        to: &'a Destination,
    },
    /// `STREAM ACCEPT`, unsilenced.
    StreamAccept {
        /// Session ID.
        id: &'a str,
    },
    /// The answer to a bridge's `PING`, echoing whatever followed it.
    Pong {
        /// Text after `PING`, as received.
        rest: &'a str,
    },
}

impl Command<'_> {
    /// The command as one line, newline included.
    pub fn line(&self) -> Result<String> {
        let mut line = match *self {
            Self::Hello => "HELLO VERSION MIN=3.1 MAX=3.3".to_string(),
            Self::DestGenerate { signature_type } => {
                format!("DEST GENERATE SIGNATURE_TYPE={signature_type}")
            }
            Self::NamingLookup { name } => format!("NAMING LOOKUP NAME={}", token(name, "name")?),
            Self::SessionCreate { id, key, options } => {
                let mut line = format!(
                    "SESSION CREATE STYLE=STREAM ID={} DESTINATION=",
                    token(id, "nickname")?
                );
                match key {
                    Key::Transient => {
                        line.push_str("TRANSIENT SIGNATURE_TYPE=");
                        line.push_str(&options.signature_type.to_string());
                    }
                    Key::Persistent(private) => {
                        line.push_str(token(private.expose_base64(), "key")?);
                    }
                }
                for (k, v) in &options.options {
                    line.push(' ');
                    line.push_str(key_token(k, "option key")?);
                    line.push('=');
                    line.push_str(token(v, "option value")?);
                }
                line
            }
            Self::StreamConnect { id, to } => format!(
                "STREAM CONNECT ID={} DESTINATION={} SILENT=false",
                token(id, "nickname")?,
                to.to_base64()
            ),
            Self::StreamAccept { id } => {
                format!("STREAM ACCEPT ID={} SILENT=false", token(id, "nickname")?)
            }
            Self::Pong { rest } => {
                if rest.contains(['\n', '\r']) {
                    return Err(Error::InvalidInput("ping text"));
                }
                format!("PONG{rest}")
            }
        };
        line.push('\n');
        Ok(line)
    }
}

/// Bytes from a bridge, split into lines. What follows the last line taken is kept, since
/// after `STREAM STATUS` (and the peer line of an accept) the connection carries data.
#[derive(Debug, Default)]
pub struct LineBuf {
    buf: Vec<u8>,
}

impl LineBuf {
    /// An empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append bytes read from the bridge.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Take the next complete line, without its newline or a trailing carriage return.
    /// `Ok(None)` means more bytes are needed.
    pub fn pop(&mut self) -> Result<Option<String>> {
        let Some(end) = self.buf.iter().position(|&b| b == b'\n') else {
            return if self.buf.len() > MAX_LINE {
                Err(Error::LineTooLong)
            } else {
                Ok(None)
            };
        };
        if end > MAX_LINE {
            return Err(Error::LineTooLong);
        }
        let mut line: Vec<u8> = self.buf.drain(..=end).collect();
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        String::from_utf8(line)
            .map(Some)
            .map_err(|e| Error::Protocol(String::from_utf8_lossy(e.as_bytes()).into()))
    }

    /// The bytes after the last line taken.
    pub fn into_rest(self) -> Vec<u8> {
        self.buf
    }
}

/// Check a reply line: `topic op` with `RESULT=OK`.
pub fn expect_ok(line: &str, topic: &str, op: &str) -> Result<Reply> {
    Reply::parse(line)?.expect(topic, op, line)?.ok()
}

/// The private key in a `DEST REPLY`, checked against its `PUB`.
pub fn dest_reply(line: &str) -> Result<PrivateKey> {
    let reply = Reply::parse(line)?.expect("DEST", "REPLY", line)?;
    let (Some(public), Some(private)) = (reply.get("PUB"), reply.get("PRIV")) else {
        return Err(Error::Protocol(line.to_string()));
    };
    let key = PrivateKey::from_base64(private)?;
    if key.destination() != &Destination::from_base64(public)? {
        return Err(Error::Protocol(
            "DEST REPLY PUB is not PRIV's destination".into(),
        ));
    }
    Ok(key)
}

/// The destination in a successful `NAMING REPLY`.
pub fn naming_reply(line: &str) -> Result<Destination> {
    let reply = expect_ok(line, "NAMING", "REPLY")?;
    let value = reply
        .get("VALUE")
        .ok_or_else(|| Error::Protocol("NAMING REPLY without VALUE".into()))?;
    Destination::from_base64(value)
}

/// The private key a successful `SESSION STATUS` reports, for a transient session.
pub fn session_reply(line: &str) -> Result<Option<String>> {
    Ok(expect_ok(line, "SESSION", "STATUS")?
        .get("DESTINATION")
        .map(str::to_string))
}

/// The peer line an accept delivers before data: the destination, then `FROM_PORT=` and
/// `TO_PORT=` from SAMv3.2 on.
pub fn accepted_peer(line: &str) -> Result<Destination> {
    let dest = line.split(' ').next().unwrap_or_default();
    Destination::from_base64(dest).map_err(|_| Error::Protocol(line.to_string()))
}

/// A process-unique session ID. Not a secret, so std's randomly keyed hasher is enough.
pub fn nickname() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    format!("truchman-{:016x}", h.finish())
}

/// `value` if it can stand as one value on a SAM line: non-empty, no whitespace, control
/// characters or quotes. `=` is allowed, since base64 pads with it and SAM splits a pair at
/// its first `=`.
fn token<'a>(value: &'a str, what: &'static str) -> Result<&'a str> {
    let clean = !value.is_empty()
        && value
            .chars()
            .all(|c| !(c.is_whitespace() || c.is_control() || c == '"'));
    if clean {
        Ok(value)
    } else {
        Err(Error::InvalidInput(what))
    }
}

/// `key` if it can stand as the key of a `KEY=VALUE` pair: a [`token`] without `=`.
fn key_token<'a>(key: &'a str, what: &'static str) -> Result<&'a str> {
    if key.contains('=') {
        return Err(Error::InvalidInput(what));
    }
    token(key, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dest::tests::sample;
    use crate::encoding::base64_encode;

    #[test]
    fn lines_split_across_reads_and_the_rest_is_kept() {
        let mut lines = LineBuf::new();
        lines.push(b"STREAM STAT");
        assert_eq!(lines.pop().unwrap(), None);
        lines.push(b"US RESULT=OK\r\npeer FROM_PORT=0\nda");
        assert_eq!(lines.pop().unwrap().unwrap(), "STREAM STATUS RESULT=OK");
        assert_eq!(lines.pop().unwrap().unwrap(), "peer FROM_PORT=0");
        assert_eq!(lines.pop().unwrap(), None);
        lines.push(b"ta");
        assert_eq!(lines.into_rest(), b"data");
    }

    #[test]
    fn a_line_longer_than_the_bound_is_refused() {
        let mut lines = LineBuf::new();
        lines.push(&[b'a'; MAX_LINE + 1]);
        assert!(matches!(lines.pop(), Err(Error::LineTooLong)));
        let mut lines = LineBuf::new();
        lines.push(&[b'a'; MAX_LINE + 1]);
        lines.push(b"\n");
        assert!(matches!(lines.pop(), Err(Error::LineTooLong)));
    }

    #[test]
    fn commands_write_the_spec_lines() {
        let (public, private) = sample(2);
        let key = PrivateKey::from_base64(&base64_encode(&private)).unwrap();
        let dest = Destination::from_bytes(public).unwrap();
        let options = SessionOptions {
            options: vec![("inbound.length".into(), "1".into())],
            ..SessionOptions::default()
        };
        assert_eq!(
            Command::Hello.line().unwrap(),
            "HELLO VERSION MIN=3.1 MAX=3.3\n"
        );
        assert_eq!(
            Command::SessionCreate {
                id: "s",
                key: &Key::Transient,
                options: &options
            }
            .line()
            .unwrap(),
            "SESSION CREATE STYLE=STREAM ID=s DESTINATION=TRANSIENT SIGNATURE_TYPE=7 inbound.length=1\n"
        );
        let persistent = Key::Persistent(key.clone());
        let line = Command::SessionCreate {
            id: "s",
            key: &persistent,
            options: &SessionOptions::default(),
        }
        .line()
        .unwrap();
        assert_eq!(
            line,
            format!(
                "SESSION CREATE STYLE=STREAM ID=s DESTINATION={}\n",
                key.expose_base64()
            )
        );
        assert_eq!(
            Command::StreamConnect { id: "s", to: &dest }
                .line()
                .unwrap(),
            format!(
                "STREAM CONNECT ID=s DESTINATION={} SILENT=false\n",
                dest.to_base64()
            )
        );
        assert_eq!(Command::Pong { rest: " 42" }.line().unwrap(), "PONG 42\n");
    }

    #[test]
    fn values_that_would_break_the_line_are_refused() {
        let bad = |options: SessionOptions| {
            Command::SessionCreate {
                id: "s",
                key: &Key::Transient,
                options: &options,
            }
            .line()
        };
        let value = SessionOptions {
            options: vec![("inbound.length".into(), "1\nSTREAM CONNECT".into())],
            ..SessionOptions::default()
        };
        let key = SessionOptions {
            options: vec![("a=b".into(), "1".into())],
            ..SessionOptions::default()
        };
        let quoted = SessionOptions {
            options: vec![("a".into(), "\"x\"".into())],
            ..SessionOptions::default()
        };
        for options in [value, key, quoted] {
            assert!(matches!(bad(options), Err(Error::InvalidInput(_))));
        }
        assert!(Command::StreamAccept { id: "a b" }.line().is_err());
        assert!(
            Command::NamingLookup {
                name: "x.i2p NAME=y"
            }
            .line()
            .is_err()
        );
        assert!(Command::NamingLookup { name: "" }.line().is_err());
        assert!(Command::Pong { rest: " 1\nSTREAM" }.line().is_err());
    }

    #[test]
    fn replies_decode_to_keys_and_peers() {
        let (public, private) = sample(4);
        let (public, private) = (base64_encode(&public), base64_encode(&private));
        let key = dest_reply(&format!("DEST REPLY PUB={public} PRIV={private}")).unwrap();
        assert_eq!(key.destination().to_base64(), public);
        let (other, _) = sample(6);
        let mismatch = format!("DEST REPLY PUB={} PRIV={private}", base64_encode(&other));
        assert!(dest_reply(&mismatch).is_err());
        let peer = accepted_peer(&format!("{public} FROM_PORT=0 TO_PORT=0")).unwrap();
        assert_eq!(&peer, key.destination());
        assert!(accepted_peer("STREAM STATUS RESULT=OK").is_err());
        assert_eq!(
            session_reply(&format!("SESSION STATUS RESULT=OK DESTINATION={private}")).unwrap(),
            Some(private)
        );
        let refused = session_reply("SESSION STATUS RESULT=DUPLICATED_ID").unwrap_err();
        assert_eq!(refused.result(), Some("DUPLICATED_ID"));
    }

    #[test]
    fn nicknames_are_unique() {
        assert_ne!(nickname(), nickname());
    }
}
