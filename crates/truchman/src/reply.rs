//! SAM reply lines: `TOPIC OP KEY=VALUE ...`, where a value may be double-quoted with `\"`
//! and `\\` escapes (SAMv3.2).

use crate::{Error, Result};

/// One parsed reply line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// First word, e.g. `HELLO`, `SESSION`, `STREAM`.
    pub topic: String,
    /// Second word, e.g. `REPLY`, `STATUS`.
    pub op: String,
    pairs: Vec<(String, Option<String>)>,
}

impl Reply {
    /// Parse one line, without its newline.
    pub fn parse(line: &str) -> Result<Self> {
        let mut words = tokens(line)?.into_iter();
        let (Some(topic), Some(op)) = (words.next(), words.next()) else {
            return Err(Error::Protocol(line.to_string()));
        };
        let pairs = words
            .map(|w| match w.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (w, None),
            })
            .collect();
        Ok(Self { topic, op, pairs })
    }

    /// The value of `key`, if present with one.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_deref())
    }

    /// This reply if it is `topic op`, else a protocol error.
    pub(crate) fn expect(self, topic: &str, op: &str, line: &str) -> Result<Self> {
        if self.topic == topic && self.op == op {
            Ok(self)
        } else {
            Err(Error::Protocol(line.to_string()))
        }
    }

    /// This reply if its `RESULT` is `OK`, else the refusal it carries.
    pub(crate) fn ok(self) -> Result<Self> {
        match self.get("RESULT") {
            Some("OK") => Ok(self),
            Some(result) => Err(Error::Refused {
                result: result.to_string(),
                message: self.get("MESSAGE").map(str::to_string),
            }),
            None => Err(Error::Protocol(format!(
                "{} {} without RESULT",
                self.topic, self.op
            ))),
        }
    }
}

/// Split on spaces outside double quotes, unquoting and unescaping quoted runs.
fn tokens(line: &str) -> Result<Vec<String>> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut quoted, mut escaped, mut started) = (false, false, false);
    for c in line.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
            started = true;
        } else if c == ' ' && !quoted {
            if started || !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
            started = false;
        } else {
            cur.push(c);
            started = true;
        }
    }
    if quoted || escaped {
        return Err(Error::Protocol(line.to_string()));
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_flags_and_quoted_values() {
        let r = Reply::parse(
            r#"SESSION STATUS RESULT=I2P_ERROR MESSAGE="tunnel \"build\" failed"  FLAG"#,
        )
        .unwrap();
        assert_eq!((r.topic.as_str(), r.op.as_str()), ("SESSION", "STATUS"));
        assert_eq!(r.get("RESULT"), Some("I2P_ERROR"));
        assert_eq!(r.get("MESSAGE"), Some(r#"tunnel "build" failed"#));
        assert_eq!(r.get("FLAG"), None);
        let err = r.ok().unwrap_err();
        assert_eq!(err.result(), Some("I2P_ERROR"));
    }

    #[test]
    fn a_value_keeps_its_own_equals_signs() {
        let r = Reply::parse("DEST REPLY PUB=abc== PRIV=de=").unwrap();
        assert_eq!(r.get("PUB"), Some("abc=="));
        assert_eq!(r.get("PRIV"), Some("de="));
    }

    #[test]
    fn unterminated_quotes_and_short_lines_are_protocol_errors() {
        assert!(matches!(
            Reply::parse(r#"HELLO REPLY MESSAGE="open"#),
            Err(Error::Protocol(_))
        ));
        assert!(matches!(Reply::parse("HELLO"), Err(Error::Protocol(_))));
        assert!(Reply::parse("HELLO REPLY").unwrap().ok().is_err());
    }
}
