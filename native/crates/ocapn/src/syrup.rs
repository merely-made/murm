// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{Limits, SessionError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Bool(bool),
    Int(i64),
    String(String),
    Symbol(String),
    Bytes(Vec<u8>),
    List(Vec<Item>),
    Map(Vec<(Item, Item)>),
    Record(String, Vec<Item>),
}
impl Item {
    pub fn record(label: &str, fields: Vec<Item>) -> Self {
        Self::Record(label.into(), fields)
    }
    pub fn symbol(s: &str) -> Self {
        Self::Symbol(s.into())
    }
    pub fn slot(n: u64) -> Result<Self, SessionError> {
        Ok(Self::Int(
            n.try_into().map_err(|_| SessionError::Limit("slot"))?,
        ))
    }
    pub fn as_slot(&self) -> Result<u64, SessionError> {
        if let Self::Int(n) = self {
            (*n).try_into()
                .map_err(|_| SessionError::Protocol("negative slot".into()))
        } else {
            Err(SessionError::Protocol("expected slot".into()))
        }
    }
    pub fn fields(&self, label: &str, count: usize) -> Result<&[Item], SessionError> {
        match self {
            Self::Record(l, fields) if l == label && fields.len() == count => Ok(fields),
            _ => Err(SessionError::Protocol(format!("expected {label}/{count}"))),
        }
    }
}

pub(crate) fn encode(item: &Item, limits: &Limits) -> Result<Vec<u8>, SessionError> {
    fn put(
        item: &Item,
        out: &mut Vec<u8>,
        limits: &Limits,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), SessionError> {
        *nodes += 1;
        if depth > limits.max_depth || *nodes > limits.max_value_nodes {
            return Err(SessionError::Limit("Syrup shape"));
        }
        let mut blob = |data: &[u8], tag: u8| -> Result<(), SessionError> {
            if data.len() > limits.max_frame_bytes {
                return Err(SessionError::Limit("scalar bytes"));
            }
            out.extend_from_slice(data.len().to_string().as_bytes());
            out.push(tag);
            out.extend_from_slice(data);
            Ok(())
        };
        match item {
            Item::Bool(b) => out.push(if *b { b't' } else { b'f' }),
            Item::Int(n) => {
                out.extend_from_slice(n.unsigned_abs().to_string().as_bytes());
                out.push(if *n >= 0 { b'+' } else { b'-' });
            },
            Item::String(s) => blob(s.as_bytes(), b'"')?,
            Item::Symbol(s) => blob(s.as_bytes(), b'\'')?,
            Item::Bytes(s) => blob(s, b':')?,
            Item::List(items) | Item::Record(_, items) => {
                if items.len() > limits.max_collection_items {
                    return Err(SessionError::Limit("collection items"));
                }
                let record = matches!(item, Item::Record(..));
                out.push(if record { b'<' } else { b'[' });
                if let Item::Record(label, _) = item {
                    put(&Item::Symbol(label.clone()), out, limits, depth + 1, nodes)?;
                }
                for value in items {
                    put(value, out, limits, depth + 1, nodes)?;
                }
                out.push(if record { b'>' } else { b']' });
            },
            Item::Map(items) => {
                if items.len() > limits.max_collection_items {
                    return Err(SessionError::Limit("collection items"));
                }
                let mut keys = Vec::with_capacity(items.len());
                for (k, v) in items {
                    let mut key = Vec::new();
                    put(k, &mut key, limits, depth + 1, nodes)?;
                    keys.push((key, v));
                }
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                if keys.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err(SessionError::Protocol("duplicate dictionary key".into()));
                }
                out.push(b'{');
                for (key, value) in keys {
                    out.extend_from_slice(&key);
                    put(value, out, limits, depth + 1, nodes)?;
                }
                out.push(b'}');
            },
        }
        if out.len() > limits.max_frame_bytes {
            return Err(SessionError::Limit("frame bytes"));
        }
        Ok(())
    }
    let mut out = Vec::new();
    put(item, &mut out, limits, 0, &mut 0)?;
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Item, SessionError> {
    struct Parser<'a> {
        bytes: &'a [u8],
        cursor: usize,
        nodes: usize,
        limits: &'a Limits,
    }
    impl Parser<'_> {
        fn error(&self) -> SessionError {
            SessionError::Protocol("invalid bounded Syrup".into())
        }
        fn take(&mut self) -> Result<u8, SessionError> {
            let b = *self.bytes.get(self.cursor).ok_or_else(|| self.error())?;
            self.cursor += 1;
            Ok(b)
        }
        fn parse(&mut self, depth: usize) -> Result<Item, SessionError> {
            self.nodes += 1;
            if depth > self.limits.max_depth || self.nodes > self.limits.max_value_nodes {
                return Err(SessionError::Limit("Syrup shape"));
            }
            let tag = self.take()?;
            match tag {
                b't' => Ok(Item::Bool(true)),
                b'f' => Ok(Item::Bool(false)),
                b'[' | b'<' | b'{' => {
                    let end = match tag {
                        b'[' => b']',
                        b'<' => b'>',
                        _ => b'}',
                    };
                    let label = if tag == b'<' {
                        match self.parse(depth + 1)? {
                            Item::Symbol(s) => Some(s),
                            _ => return Err(self.error()),
                        }
                    } else {
                        None
                    };
                    let mut items = Vec::new();
                    while self.bytes.get(self.cursor) != Some(&end) {
                        let max = self
                            .limits
                            .max_collection_items
                            .saturating_mul(if tag == b'{' { 2 } else { 1 });
                        if items.len() >= max {
                            return Err(SessionError::Limit("collection items"));
                        }
                        items.push(self.parse(depth + 1)?);
                    }
                    self.cursor += 1;
                    if tag == b'{' {
                        if items.len() % 2 != 0 {
                            return Err(self.error());
                        }
                        let mut map = Vec::with_capacity(items.len() / 2);
                        let mut iter = items.into_iter();
                        let mut previous: Option<Vec<u8>> = None;
                        while let Some(key) = iter.next() {
                            let encoded = encode(&key, self.limits)?;
                            if previous.as_ref().is_some_and(|p| p >= &encoded) {
                                return Err(self.error());
                            }
                            previous = Some(encoded);
                            map.push((key, iter.next().ok_or_else(|| self.error())?));
                        }
                        Ok(Item::Map(map))
                    } else if let Some(label) = label {
                        Ok(Item::Record(label, items))
                    } else {
                        Ok(Item::List(items))
                    }
                },
                b'0'..=b'9' => {
                    let start = self.cursor - 1;
                    while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                        self.cursor += 1;
                        if self.cursor - start > 20 {
                            return Err(self.error());
                        }
                    }
                    let digits = &self.bytes[start..self.cursor];
                    if digits.len() > 1 && digits[0] == b'0' {
                        return Err(self.error());
                    }
                    let number: u64 = std::str::from_utf8(digits)
                        .map_err(|_| self.error())?
                        .parse()
                        .map_err(|_| self.error())?;
                    let separator = self.take()?;
                    if separator == b'+' || separator == b'-' {
                        let signed = if separator == b'+' {
                            i64::try_from(number).map_err(|_| self.error())?
                        } else {
                            if number == 0 {
                                return Err(self.error());
                            }
                            i64::try_from(-(i128::from(number))).map_err(|_| self.error())?
                        };
                        return Ok(Item::Int(signed));
                    }
                    if !matches!(separator, b':' | b'"' | b'\'') {
                        return Err(self.error());
                    }
                    let length: usize = number.try_into().map_err(|_| self.error())?;
                    if length > self.limits.max_frame_bytes {
                        return Err(SessionError::Limit("scalar bytes"));
                    }
                    let end = self
                        .cursor
                        .checked_add(length)
                        .ok_or_else(|| self.error())?;
                    let data = self
                        .bytes
                        .get(self.cursor..end)
                        .ok_or_else(|| self.error())?;
                    self.cursor = end;
                    if separator == b':' {
                        Ok(Item::Bytes(data.to_vec()))
                    } else {
                        let s = std::str::from_utf8(data)
                            .map_err(|_| self.error())?
                            .to_owned();
                        Ok(if separator == b'"' {
                            Item::String(s)
                        } else {
                            Item::Symbol(s)
                        })
                    }
                },
                _ => Err(self.error()),
            }
        }
    }
    if bytes.len() > limits.max_frame_bytes {
        return Err(SessionError::Limit("frame bytes"));
    }
    let mut parser = Parser {
        bytes,
        cursor: 0,
        nodes: 0,
        limits,
    };
    let item = parser.parse(0)?;
    if parser.cursor != bytes.len() {
        return Err(parser.error());
    }
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_and_bounded() {
        let l = Limits::default();
        let value = Item::record(
            "example",
            vec![
                Item::Int(-1),
                Item::String("héllo".into()),
                Item::Map(vec![
                    (Item::symbol("z"), Item::Bool(false)),
                    (Item::symbol("a"), Item::Bytes(vec![0, 255])),
                ]),
            ],
        );
        let wire = encode(&value, &l).unwrap();
        let decoded = decode(&wire, &l).unwrap();
        assert_eq!(encode(&decoded, &l).unwrap(), wire);
        for bytes in [
            b"01+".as_slice(),
            b"0-",
            b"9:short",
            b"[]f",
            b"<1\"x>",
            b"{1'a0+1'a1+}",
        ] {
            assert!(decode(bytes, &l).is_err(), "{bytes:?}");
        }
        let mut tiny = l.clone();
        tiny.max_depth = 2;
        assert!(decode(b"[[[[]]]]", &tiny).is_err());
        assert!(decode(b"999999999999999999999999:", &l).is_err());
    }
}
