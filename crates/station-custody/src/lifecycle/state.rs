//! Controller state that survives between invocations: the outer replay counter and the
//! mutation sequence, each the highest value this controller has ever offered a board.
//!
//! Both are spent before the board can see them. A board may accept and journal a command
//! whose reply is then lost, so a value is written durably (synced temporary file, then a
//! rename over the record) before the frame carrying it is sent, and never rewound. A crash
//! at any point leaves either the old record or the new one; at worst one value is skipped,
//! which the board's window allows, and none is offered twice.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The values a board checks on every signed command, kept by the controller.
pub trait ControllerState {
    /// The next outer replay counter, already durable when this returns.
    ///
    /// # Errors
    /// The record could not be read, parsed, advanced or made durable; nothing is sent then.
    fn reserve_counter(&mut self) -> io::Result<u64>;
    /// The next mutation sequence, already durable when this returns. Only mutations (apply,
    /// commit, revert) spend one; status does not.
    ///
    /// # Errors
    /// As for [`Self::reserve_counter`].
    fn reserve_sequence(&mut self) -> io::Result<u64>;
}

/// One record: the highest value ever offered. The counter's file format is the one
/// `station-claim status` has always written.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
struct Spent {
    last_used: u64,
}

/// The two records beside the controller scope in the authority root.
pub struct FileControllerState {
    root: PathBuf,
}

impl FileControllerState {
    pub fn new(authority_root: impl Into<PathBuf>) -> Self {
        Self {
            root: authority_root.into(),
        }
    }

    pub fn counter_path(&self) -> PathBuf {
        self.root.join("first-owner-controller-counter.json")
    }

    pub fn sequence_path(&self) -> PathBuf {
        self.root.join("first-owner-controller-sequence.json")
    }
}

impl ControllerState for FileControllerState {
    fn reserve_counter(&mut self) -> io::Result<u64> {
        reserve(&self.counter_path(), "counter")
    }

    fn reserve_sequence(&mut self) -> io::Result<u64> {
        reserve(&self.sequence_path(), "mutation sequence")
    }
}

/// Read a record, advance it by one, and make the advance durable before returning. A
/// missing record starts at one: a freshly claimed board's grant has accepted nothing yet.
fn reserve(path: &Path, what: &str) -> io::Result<u64> {
    let last_used = match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str::<Spent>(&text)
                .map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "invalid first-owner controller {what} at {}: {error}",
                            path.display()
                        ),
                    )
                })?
                .last_used
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let next = last_used.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("first-owner controller {what} is exhausted"),
        )
    })?;
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec(&Spent { last_used: next })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    file.write_all(&contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_value_is_durable_before_use_and_never_reoffered() {
        let root = tempfile::tempdir().unwrap();
        let mut state = FileControllerState::new(root.path());
        assert_eq!(state.reserve_counter().unwrap(), 1);
        assert_eq!(state.reserve_counter().unwrap(), 2);
        assert_eq!(state.reserve_sequence().unwrap(), 1);
        // A fresh handle, as the next invocation would have, continues past what was spent.
        let mut again = FileControllerState::new(root.path());
        assert_eq!(again.reserve_counter().unwrap(), 3);
        assert_eq!(again.reserve_sequence().unwrap(), 2);
        assert!(!state.counter_path().with_extension("json.tmp").exists());
        std::fs::write(state.sequence_path(), b"not json").unwrap();
        assert!(again.reserve_sequence().is_err());
    }
}
