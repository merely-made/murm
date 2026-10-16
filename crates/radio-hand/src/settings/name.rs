//! Public, board-local labels. Independent of radio addresses and host identities.

use super::Settings;
use radio_face::Text;

pub const MAX_NAME_LEN: usize = 16;

/// A displayable name with no control bytes or silent truncation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceName(Text<MAX_NAME_LEN>);

impl DeviceName {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_NAME_LEN
            || bytes.iter().any(|byte| !(b' '..=b'~').contains(byte))
            || bytes.first() == Some(&b' ')
            || bytes.last() == Some(&b' ')
        {
            return None;
        }
        Text::try_from_bytes(bytes).ok().map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameCommand {
    Report,
    Set(DeviceName),
    Invalid,
}

impl NameCommand {
    /// Recognize invalid name commands too, so they never become radio data.
    pub fn parse(packet: &[u8]) -> Option<Self> {
        let line = packet
            .strip_suffix(b"\r\n")
            .or_else(|| packet.strip_suffix(b"\n"))?;
        match line {
            b"name" => Some(Self::Report),
            b"name clear" => Some(Self::Set(DeviceName::default())),
            _ if line.starts_with(b"name ") => Some(
                line.strip_prefix(b"name set ")
                    .filter(|name| !name.is_empty())
                    .and_then(DeviceName::parse)
                    .map(Self::Set)
                    .unwrap_or(Self::Invalid),
            ),
            _ => None,
        }
    }
}

pub fn report(settings: Option<Settings>) -> Text<64> {
    use core::fmt::Write as _;
    let mut reply = Text::empty();
    match settings {
        Some(settings) => {
            let _ = write!(&mut reply, "name={}\r\n", settings.device_name.as_str());
        }
        None => {
            let _ = write!(&mut reply, "name unavailable\r\n");
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fit_both_displays_and_reject_invisible_or_truncated_labels() {
        for value in [b"SB1".as_slice(), b"Board 2", b"1234567890123456", b""] {
            assert_eq!(DeviceName::parse(value).unwrap().as_str().as_bytes(), value);
        }
        for value in [
            b"12345678901234567".as_slice(),
            b" SB1",
            b"SB1 ",
            b"SB\n1",
            b"SB\x7f1",
            b"\xc3\xa9",
        ] {
            assert!(DeviceName::parse(value).is_none());
        }
    }

    #[test]
    fn malformed_commands_are_consumed_without_a_settings_write() {
        for value in [
            b"name set\n".as_slice(),
            b"name set \n",
            b"name set SB\t1\n",
            b"name unknown\n",
        ] {
            assert_eq!(NameCommand::parse(value), Some(NameCommand::Invalid));
        }
        assert_eq!(NameCommand::parse(b"name\r\n"), Some(NameCommand::Report));
        assert_eq!(
            NameCommand::parse(b"name clear\n"),
            Some(NameCommand::Set(DeviceName::default()))
        );
        assert_eq!(
            NameCommand::parse(b"name set SB2\n"),
            Some(NameCommand::Set(DeviceName::parse(b"SB2").unwrap()))
        );
        assert_eq!(NameCommand::parse(b"other\n"), None);
    }
}
