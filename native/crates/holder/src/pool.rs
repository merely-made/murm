// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! The pool setting: which holders a site replicates to, and what each route
//! shows them.
//!
//! One stack-level setting selects among the personal mesh (the user's own
//! devices) and named holder pools. Every route carries a privacy tier in two
//! parts: the data it exposes (plain release content or only a sealed payload)
//! and the resolution it gives the holder about the requester (a network
//! address, a mesh pseudonym, or nothing linkable). A route below the
//! configured floor is refused and reported, never dropped silently.
//!
//! Relaying is not here. Moot-operated relay pools belong to Gemot's relay
//! pools plan; a named pool here is a list of holders, not a relay service.

use crate::dial::HolderTarget;

/// What the holder learns about whoever dials it, worst first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resolution {
    /// The requester's network address or endpoint (iroh, Noise over TCP).
    NetworkAddress,
    /// A mesh destination or pseudonym, no network address (Reticulum).
    MeshPseudonym,
    /// Nothing that links to the requester (onion, I2P), or no network at all.
    Hidden,
}

/// What the holder can read of the release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataVisible {
    /// Public release content.
    Plain,
    /// Only a sealed payload; the holder cannot read it.
    Sealed,
}

/// The two parts of a route's privacy tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyTier {
    pub data: DataVisible,
    pub resolution: Resolution,
}

/// The carrier a route runs over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarrierClass {
    InProcess,
    Iroh,
    NoiseTcp,
    Reticulum,
    I2p,
    Onion,
}

impl CarrierClass {
    /// What a holder on this carrier learns about the requester.
    pub fn resolution(self) -> Resolution {
        match self {
            Self::Iroh | Self::NoiseTcp => Resolution::NetworkAddress,
            Self::Reticulum => Resolution::MeshPseudonym,
            Self::I2p | Self::Onion | Self::InProcess => Resolution::Hidden,
        }
    }
}

/// A pool's identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoolId {
    /// The user's own devices.
    PersonalMesh,
    /// A named pool of holders someone operates.
    Named(String),
}

/// One holder and the carrier that reaches it.
#[derive(Clone, Copy, Debug)]
pub struct PoolRoute {
    pub target: HolderTarget,
    pub class: CarrierClass,
}

/// A set of holders.
#[derive(Clone, Debug)]
pub struct Pool {
    pub id: PoolId,
    pub routes: Vec<PoolRoute>,
}

/// The stack setting.
#[derive(Clone, Debug)]
pub struct PoolSetting {
    /// Known pools.
    pub pools: Vec<Pool>,
    /// Pools to replicate to, in preference order.
    pub selected: Vec<PoolId>,
    /// The least private resolution the user accepts.
    pub floor: Resolution,
}

/// A route chosen, with the tier it runs at.
#[derive(Clone, Copy, Debug)]
pub struct Planned<'a> {
    pub pool: &'a PoolId,
    pub route: &'a PoolRoute,
    pub tier: PrivacyTier,
}

/// A route refused, and why.
#[derive(Clone, Copy, Debug)]
pub struct Refused<'a> {
    pub pool: &'a PoolId,
    pub route: &'a PoolRoute,
    pub tier: PrivacyTier,
}

/// The outcome of applying the setting to one release.
#[derive(Debug, Default)]
pub struct Plan<'a> {
    pub routes: Vec<Planned<'a>>,
    /// Routes below the floor, and any selected pool that is not known.
    pub below_floor: Vec<Refused<'a>>,
    pub unknown_pools: Vec<&'a PoolId>,
}

impl Plan<'_> {
    /// The holders to configure for acknowledgement.
    pub fn targets(&self) -> Vec<HolderTarget> {
        self.routes
            .iter()
            .map(|planned| planned.route.target)
            .collect()
    }
}

impl PoolSetting {
    /// Apply the setting to a release that is `sealed` or plain.
    pub fn plan(&self, sealed: bool) -> Plan<'_> {
        let data = if sealed {
            DataVisible::Sealed
        } else {
            DataVisible::Plain
        };
        let mut plan = Plan::default();
        for id in &self.selected {
            let Some(pool) = self.pools.iter().find(|pool| pool.id == *id) else {
                plan.unknown_pools.push(id);
                continue;
            };
            for route in &pool.routes {
                let tier = PrivacyTier {
                    data,
                    resolution: route.class.resolution(),
                };
                if tier.resolution >= self.floor {
                    plan.routes.push(Planned {
                        pool: &pool.id,
                        route,
                        tier,
                    });
                } else {
                    plan.below_floor.push(Refused {
                        pool: &pool.id,
                        route,
                        tier,
                    });
                }
            }
        }
        plan
    }
}
