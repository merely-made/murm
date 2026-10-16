//! Host-durable exclusive Sennet nonce intervals. Initialization is a separate
//! explicit act; connection/reconnect never recreates missing allocator state.
use super::{FileJournal, RadioJournal};
use sennet::instance::PacketIdLease;
use serde::{Deserialize, Serialize};
use std::{
    io,
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn fence_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".initialized");
    name.into()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u8,
    realm: String,
    source: u32,
    next: u32,
}

/// Holds the file's exclusive lock for the entire connection, including all
/// unused IDs. A crashed connection abandons its whole reserved interval.
pub struct SennetPacketIds {
    _journal: FileJournal,
}
impl SennetPacketIds {
    /// Explicit first commissioning. Refuses any existing allocator.
    pub fn initialize(path: &Path, realm: &str, source: u32) -> io::Result<()> {
        if realm.is_empty()
            || realm.len() > 128
            || !realm
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || matches!(source, 0 | u32::MAX)
        {
            return Err(io::Error::other("invalid Sennet packet-ID scope"));
        }
        let mut journal = FileJournal::new(path);
        if journal.load()?.is_some() {
            return Err(io::Error::other(
                "preserve existing Sennet packet-ID allocator",
            ));
        }
        let bytes = serde_json::to_vec(&State {
            version: 1,
            realm: realm.into(),
            source,
            next: 1,
        })?;
        // Persist a separate create-once commissioning fence first. Losing
        // allocator data cannot turn initialization into a same-identity reset.
        let mut fence = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(fence_path(path))?;
        fence.write_all(&bytes)?;
        fence.sync_all()?;
        journal.commit(&bytes)
    }
    /// Persist the next high-water mark before releasing any IDs for use.
    /// State corruption, loss, wrong scope, exhaustion or another owner refuses.
    pub fn reserve(
        path: &Path,
        realm: &str,
        source: u32,
        count: u32,
    ) -> io::Result<(Self, PacketIdLease)> {
        let mut journal = FileJournal::new(path);
        let bytes = journal.load()?.ok_or_else(|| {
            io::Error::other("Sennet packet-ID state missing; explicit commissioning required")
        })?;
        let mut state: State = serde_json::from_slice(&bytes)?;
        let mut fence_bytes = Vec::new();
        std::fs::File::open(fence_path(path))?
            .take(1025)
            .read_to_end(&mut fence_bytes)?;
        if fence_bytes.len() > 1024 {
            return Err(io::Error::other("Sennet commissioning fence exceeds bound"));
        }
        let fence: State = serde_json::from_slice(&fence_bytes)?;
        if fence.version != 1 || fence.realm != realm || fence.source != source || fence.next != 1 {
            return Err(io::Error::other("Sennet commissioning fence mismatch"));
        }
        if state.version != 1
            || state.realm != realm
            || state.source != source
            || state.next == 0
            || count == 0
        {
            return Err(io::Error::other(
                "Sennet packet-ID scope or bounds mismatch",
            ));
        }
        let start = state.next;
        let end = start
            .checked_add(count)
            .ok_or_else(|| io::Error::other("Sennet packet-ID space exhausted"))?;
        let lease = PacketIdLease::new(source, start, end, start).map_err(io::Error::other)?;
        state.next = end;
        journal.commit(&serde_json::to_vec(&state)?)?;
        Ok((Self { _journal: journal }, lease))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reopen_abandons_unused_ids_and_refuses_a_second_owner_or_scope() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ids.json");
        SennetPacketIds::initialize(&p, "ridge", 7).unwrap();
        let (held, a) = SennetPacketIds::reserve(&p, "ridge", 7, 256).unwrap();
        assert_eq!((a.start(), a.end()), (1, 257));
        assert!(SennetPacketIds::reserve(&p, "ridge", 7, 256).is_err());
        drop(held);
        assert!(SennetPacketIds::initialize(&p, "ridge", 7).is_err());
        assert!(SennetPacketIds::reserve(&p, "valley", 7, 256).is_err());
        assert!(SennetPacketIds::reserve(&p, "ridge", 8, 256).is_err());
        let (_, b) = SennetPacketIds::reserve(&p, "ridge", 7, 256).unwrap();
        assert_eq!((b.start(), b.end()), (257, 513));
    }
    #[test]
    fn lost_allocator_cannot_be_reinitialized_under_the_existing_commissioning_fence() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ids.json");
        SennetPacketIds::initialize(&p, "ridge", 7).unwrap();
        std::fs::remove_file(&p).unwrap();
        assert!(SennetPacketIds::initialize(&p, "ridge", 7).is_err());
        assert!(SennetPacketIds::reserve(&p, "ridge", 7, 256).is_err());
    }
    #[test]
    fn missing_corrupt_and_exhausted_state_refuse_without_repair() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ids.json");
        assert!(SennetPacketIds::reserve(&p, "ridge", 7, 256).is_err());
        assert!(!p.exists());
        std::fs::write(&p, b"torn").unwrap();
        assert!(SennetPacketIds::reserve(&p, "ridge", 7, 256).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"torn");
        std::fs::remove_file(&p).unwrap();
        SennetPacketIds::initialize(&p, "ridge", 7).unwrap();
        std::fs::write(
            &p,
            serde_json::to_vec(&State {
                version: 1,
                realm: "ridge".into(),
                source: 7,
                next: u32::MAX - 1,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(SennetPacketIds::reserve(&p, "ridge", 7, 256).is_err());
    }
}
