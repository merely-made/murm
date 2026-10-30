// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{
    Limits, SessionError,
    syrup::{self, Item},
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    OcapnSuite31f0b806,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerLocator {
    pub transport: String,
    pub designator: String,
    pub hints: Vec<(String, String)>,
}
impl PeerLocator {
    pub fn new(transport: impl Into<String>, designator: impl Into<String>) -> Self {
        Self {
            transport: transport.into(),
            designator: designator.into(),
            hints: Vec::new(),
        }
    }
    pub(crate) fn item(&self) -> Item {
        Item::record(
            "ocapn-peer",
            vec![
                Item::Symbol(self.transport.clone()),
                Item::String(self.designator.clone()),
                Item::Map(
                    self.hints
                        .iter()
                        .map(|(k, v)| (Item::String(k.clone()), Item::String(v.clone())))
                        .collect(),
                ),
            ],
        )
    }
    pub(crate) fn parse(item: &Item) -> Result<Self, SessionError> {
        let f = item.fields("ocapn-peer", 3)?;
        let (Item::Symbol(transport), Item::String(designator)) = (&f[0], &f[1]) else {
            return Err(protocol("invalid peer locator"));
        };
        let hints = match &f[2] {
            Item::Map(xs) => xs
                .iter()
                .map(|(k, v)| {
                    let (Item::String(k), Item::String(v)) = (k, v) else {
                        return Err(protocol("locator hint types"));
                    };
                    Ok((k.clone(), v.clone()))
                })
                .collect::<Result<_, _>>()?,
            Item::Bool(false) => Vec::new(),
            _ => return Err(protocol("invalid hints")),
        };
        if transport.is_empty() || transport.contains('.') || designator.is_empty() {
            return Err(protocol("invalid locator identity"));
        }
        Ok(Self {
            transport: transport.clone(),
            designator: designator.clone(),
            hints,
        })
    }
    pub(crate) fn same_peer(&self, other: &Self) -> bool {
        self.transport == other.transport && self.designator == other.designator
    }
}
pub(crate) fn protocol(message: &str) -> SessionError {
    SessionError::Protocol(message.into())
}
fn list(items: Vec<Item>) -> Item {
    Item::List(items)
}
pub(crate) fn start(
    key: &SigningKey,
    location: &PeerLocator,
    limits: &Limits,
) -> Result<Item, SessionError> {
    let signed = syrup::encode(&Item::record("my-location", vec![location.item()]), limits)?;
    let signature = key.sign(&signed).to_bytes();
    let public = list(vec![
        Item::symbol("public-key"),
        list(vec![
            Item::symbol("ecc"),
            list(vec![Item::symbol("curve"), Item::symbol("Ed25519")]),
            list(vec![Item::symbol("flags"), Item::symbol("eddsa")]),
            list(vec![
                Item::symbol("q"),
                Item::Bytes(key.verifying_key().to_bytes().to_vec()),
            ]),
        ]),
    ]);
    let signature = list(vec![
        Item::symbol("sig-val"),
        list(vec![
            Item::symbol("eddsa"),
            list(vec![
                Item::symbol("r"),
                Item::Bytes(signature[..32].to_vec()),
            ]),
            list(vec![
                Item::symbol("s"),
                Item::Bytes(signature[32..].to_vec()),
            ]),
        ]),
    ]);
    Ok(Item::record(
        "op:start-session",
        vec![
            Item::String("1.0".into()),
            public,
            location.item(),
            signature,
        ],
    ))
}
pub(crate) fn verify_start(
    item: &Item,
    expected: Option<&PeerLocator>,
    limits: &Limits,
) -> Result<PeerLocator, SessionError> {
    let f = item.fields("op:start-session", 4)?;
    if f[0] != Item::String("1.0".into()) {
        return Err(protocol("unsupported CapTP version"));
    }
    let Item::List(public) = &f[1] else {
        return Err(protocol("invalid public key"));
    };
    let [tag, Item::List(ecc)] = public.as_slice() else {
        return Err(protocol("invalid public key"));
    };
    if *tag != Item::symbol("public-key") {
        return Err(protocol("invalid public key"));
    }
    let [curve_tag, curve, flags, Item::List(q)] = ecc.as_slice() else {
        return Err(protocol("invalid public key"));
    };
    if *curve_tag != Item::symbol("ecc")
        || *curve != list(vec![Item::symbol("curve"), Item::symbol("Ed25519")])
        || *flags != list(vec![Item::symbol("flags"), Item::symbol("eddsa")])
    {
        return Err(protocol("unsupported key encoding"));
    }
    let [qtag, Item::Bytes(bytes)] = q.as_slice() else {
        return Err(protocol("invalid public key"));
    };
    if *qtag != Item::symbol("q") {
        return Err(protocol("invalid public key"));
    }
    let bytes: &[u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| protocol("invalid public key length"))?;
    let key = VerifyingKey::from_bytes(bytes).map_err(|_| protocol("invalid public key"))?;
    let Item::List(sig) = &f[3] else {
        return Err(protocol("invalid signature"));
    };
    let [tag, Item::List(eddsa)] = sig.as_slice() else {
        return Err(protocol("invalid signature"));
    };
    let [algorithm, Item::List(r), Item::List(s)] = eddsa.as_slice() else {
        return Err(protocol("invalid signature"));
    };
    let [rtag, Item::Bytes(r)] = r.as_slice() else {
        return Err(protocol("invalid signature"));
    };
    let [stag, Item::Bytes(s)] = s.as_slice() else {
        return Err(protocol("invalid signature"));
    };
    if *tag != Item::symbol("sig-val")
        || *algorithm != Item::symbol("eddsa")
        || *rtag != Item::symbol("r")
        || *stag != Item::symbol("s")
        || r.len() != 32
        || s.len() != 32
    {
        return Err(protocol("invalid signature"));
    }
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(r);
    signature[32..].copy_from_slice(s);
    let location = PeerLocator::parse(&f[2])?;
    if expected.is_some_and(|e| !e.same_peer(&location)) {
        return Err(protocol("admitted locator mismatch"));
    }
    let signed = syrup::encode(&Item::record("my-location", vec![f[2].clone()]), limits)?;
    key.verify_strict(&signed, &Signature::from_bytes(&signature))
        .map_err(|_| protocol("invalid location signature"))?;
    Ok(location)
}
