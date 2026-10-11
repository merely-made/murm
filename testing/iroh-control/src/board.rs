// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
//! Real control runtime and A/B codec; simulated quiet owner and configuration applier.

use std::{
    convert::Infallible,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, ensure};
use heapless::Vec;
use radio_hand::{control::*, store::Slot};
use retinue::identity::PrivateIdentity;
use seneschal::region::Region;
use serde::Serialize;

pub const NODE: NodeId = NodeId([0x10; 16]);
const PAGE: usize = 4096;

/// Public test fixture only. Never use this key or the semantic key for commissioning.
pub fn owner() -> PrivateIdentity {
    PrivateIdentity::from_secret_bytes(&[0x30; 64])
}

pub fn public(frequency: u32) -> PublicConfigurationV1 {
    PublicConfigurationV1::new(
        Region::Us915,
        selvage::PhyProfile::meshtastic_long_fast(frequency),
        ReticulumTransportPolicy::new(false, false, 0).unwrap(),
        ManagementCarrierSet::from_mask(0b1001).unwrap(),
    )
    .unwrap()
}

fn facts() -> BoardRecoveryFacts {
    BoardRecoveryFacts::new(
        Vec::from_slice(&[
            RecoveryPathFacts::new(ManagementCarrier::Usb, true, false, false).unwrap(),
            RecoveryPathFacts::new(ManagementCarrier::Ip, false, true, true).unwrap(),
        ])
        .unwrap(),
    )
    .unwrap()
}

fn initial() -> DurableState {
    DurableState::new(
        NODE,
        Vec::from_slice(&[OwnerGrant::from_retinue_identity(
            owner().public(),
            ControllerRole::Owner,
        )])
        .unwrap(),
        ConfigGeneration(7),
        DurableConfig {
            public: public(906_875_000),
            sealed_credentials: Vec::new(),
        },
        RecoveryPolicy::new(
            RecoveryClause::new(ManagementCarrierSet::from_mask(1).unwrap(), 1).unwrap(),
            RecoveryClause::disabled(),
        )
        .unwrap(),
        &facts(),
    )
    .unwrap()
}

pub struct FileStore {
    directory: PathBuf,
}

impl FileStore {
    fn path(&self, slot: Slot) -> PathBuf {
        self.directory.join(match slot {
            Slot::A => "a.bin",
            Slot::B => "b.bin",
        })
    }

    fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let store = Self {
            directory: directory.to_owned(),
        };
        let a_exists = store.path(Slot::A).exists();
        let b_exists = store.path(Slot::B).exists();
        ensure!(
            a_exists == b_exists,
            "incomplete A/B fixture; refusing reseed"
        );
        if !a_exists {
            let mut body = [0; MAX_DURABLE_BODY];
            let mut page = [0xff; PAGE];
            let write = next_record(
                &[0xff; PAGE],
                &[0xff; PAGE],
                &initial(),
                &mut body,
                &mut page,
            )
            .map_err(|e| anyhow!("seed: {e:?}"))?;
            for slot in [Slot::A, Slot::B] {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(store.path(slot))?;
                file.write_all(&[0xff; PAGE])?;
                file.sync_all()?;
            }
            let mut seeded = store;
            seeded.program_slot(write.slot, &page[..write.len])?;
            File::open(directory)?.sync_all()?;
            return Ok(seeded);
        }
        Ok(store)
    }
}

impl AbSlotStore for FileStore {
    type Error = std::io::Error;
    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> std::io::Result<()> {
        let mut file = File::open(self.path(slot))?;
        if file.metadata()?.len() != PAGE as u64 || out.len() != PAGE {
            return Err(std::io::Error::other("wrong journal page length"));
        }
        file.read_exact(out)
    }
    fn erase_slot(&mut self, slot: Slot) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.path(slot))?;
        file.write_all(&[0xff; PAGE])?;
        file.sync_all()
    }
    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> std::io::Result<()> {
        if record.len() > PAGE {
            return Err(std::io::Error::other("oversized journal write"));
        }
        let mut file = OpenOptions::new().write(true).open(self.path(slot))?;
        file.write_all(record)?;
        file.sync_all()
    }
}

#[derive(Default, Serialize)]
pub struct SimApplier {
    pub calls: usize,
}
impl ConfigApplier for SimApplier {
    type Error = Infallible;
    async fn apply(&mut self, _: &DurableConfig) -> Result<(), Infallible> {
        self.calls += 1;
        Ok(())
    }
}

pub struct SimOwner {
    store: FileStore,
    applier: SimApplier,
    quiet_entries: usize,
    quiet_exits: usize,
    aborted: bool,
}
pub struct SimGuard<'a>(&'a mut SimOwner);
impl QuietWindow for SimOwner {
    type Error = Infallible;
    type StoreError = std::io::Error;
    type ApplyError = Infallible;
    type Guard<'a> = SimGuard<'a>;
    async fn enter(&mut self) -> Result<SimGuard<'_>, Infallible> {
        self.quiet_entries += 1;
        Ok(SimGuard(self))
    }
}
impl QuietGuard for SimGuard<'_> {
    type Error = Infallible;
    fn abort(&mut self) {
        self.0.aborted = true;
    }
    async fn finish(&mut self) -> Result<QuietExit, Infallible> {
        self.0.quiet_exits += 1;
        Ok(QuietExit::Resumed)
    }
}
impl AbSlotStore for SimGuard<'_> {
    type Error = std::io::Error;
    fn read_slot(&mut self, slot: Slot, out: &mut [u8]) -> std::io::Result<()> {
        self.0.store.read_slot(slot, out)
    }
    fn erase_slot(&mut self, slot: Slot) -> std::io::Result<()> {
        self.0.store.erase_slot(slot)
    }
    fn program_slot(&mut self, slot: Slot, record: &[u8]) -> std::io::Result<()> {
        self.0.store.program_slot(slot, record)
    }
}
impl ConfigApplier for SimGuard<'_> {
    type Error = Infallible;
    async fn apply(&mut self, config: &DurableConfig) -> Result<(), Infallible> {
        self.0.applier.apply(config).await
    }
}

pub struct Board {
    runtime: ControlRuntime,
    owner: SimOwner,
    directory: PathBuf,
    a: [u8; PAGE],
    b: [u8; PAGE],
    body: [u8; MAX_DURABLE_BODY],
    page: [u8; PAGE],
}

impl Board {
    pub async fn open(directory: &Path) -> Result<Self> {
        let mut board = Self {
            // SAFETY: a fresh host process models a reset before any simulated radio work.
            // This is not a claim about a physical hardware reset or quiet-window behavior.
            runtime: unsafe {
                ControlRuntime::new_after_hardware_reset(
                    NODE,
                    SemanticTagKey::from_bytes([0x80; 32]),
                    facts(),
                )
            },
            owner: SimOwner {
                store: FileStore::open(directory)?,
                applier: SimApplier::default(),
                quiet_entries: 0,
                quiet_exits: 0,
                aborted: false,
            },
            directory: directory.to_owned(),
            a: [0; PAGE],
            b: [0; PAGE],
            body: [0; MAX_DURABLE_BODY],
            page: [0; PAGE],
        };
        let mut scratch =
            DurableScratch::new(&mut board.a, &mut board.b, &mut board.body, &mut board.page)
                .unwrap();
        board
            .runtime
            .boot_pre_radio(
                &mut board.owner.store,
                &mut board.owner.applier,
                &mut scratch,
            )
            .await
            .map_err(|e| anyhow!("boot: {e:?}"))?;
        board.record("boot")?;
        Ok(board)
    }

    pub async fn command(&mut self, wire: &[u8], now: u64) -> Result<Response> {
        let mut verifier = restore_control_verifier(self.runtime.state().unwrap())
            .map_err(|e| anyhow!("restore: {e:?}"))?;
        let verified = verifier.verify(wire).map_err(|e| anyhow!("outer: {e:?}"))?;
        let mut scratch =
            DurableScratch::new(&mut self.a, &mut self.b, &mut self.body, &mut self.page).unwrap();
        let inbound = match decode_verified_command(&verified) {
            Ok(inbound) => inbound,
            Err(error) => {
                self.runtime
                    .record_verified_command(&mut self.owner, &mut scratch, &verified)
                    .await
                    .map_err(|e| anyhow!("journal malformed: {e:?}"))?;
                self.record("malformed-inner")?;
                return Err(anyhow!("inner: {error:?}"));
            }
        };
        let mut token = [0; COMMIT_TOKEN_LEN];
        getrandom::fill(&mut token).map_err(|e| anyhow!("entropy: {e}"))?;
        let result = self
            .runtime
            .serve_inbound(
                &mut self.owner,
                &mut scratch,
                &inbound,
                now,
                FirstWriteStatus {
                    control: PairEvidence::Valid,
                    pending: PairEvidence::Blank,
                },
                token,
            )
            .await
            .map_err(|e| anyhow!("serve: {e:?}"));
        self.record("command")?;
        result.map(LiveOutcome::into_value)
    }

    pub async fn expire(&mut self, now: u64) -> Result<()> {
        if self
            .runtime
            .provisional_deadline_ms()
            .is_some_and(|deadline| now >= deadline)
        {
            let mut scratch =
                DurableScratch::new(&mut self.a, &mut self.b, &mut self.body, &mut self.page)
                    .unwrap();
            let result = self
                .runtime
                .expire(&mut self.owner, &mut scratch, now)
                .await
                .map_err(|e| anyhow!("expire: {e:?}"))?;
            ensure!(*result.value(), "deadline did not roll back");
            self.record("expiry")?;
        }
        Ok(())
    }

    fn record(&self, event: &str) -> Result<()> {
        ensure!(
            !self.owner.aborted && self.owner.quiet_entries == self.owner.quiet_exits,
            "simulated quiet ownership lost"
        );
        let state = self.runtime.state().unwrap();
        fs::write(
            self.directory.join("snapshot.tmp"),
            serde_json::to_vec(&serde_json::json!({
                "event": event, "generation": state.known_good().generation.0,
                "provisional": state.provisional().is_some(), "outer_counter": state.owner_grants()[0].accepted_outer_counter(),
                "applies": self.owner.applier.calls, "quiet_entries": self.owner.quiet_entries,
                "quiet_exits": self.owner.quiet_exits, "recovered_rollback": self.runtime.recovered_rollback(),
            }))?,
        )?;
        fs::rename(
            self.directory.join("snapshot.tmp"),
            self.directory.join("snapshot.json"),
        )?;
        Ok(())
    }
}

pub fn read_state(directory: &Path) -> Result<DurableState> {
    let a = fs::read(directory.join("a.bin"))?;
    let b = fs::read(directory.join("b.bin"))?;
    load(&a, &b).map_err(|e| anyhow!("read journal: {e:?}"))
}
