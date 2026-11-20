//! Where the board-minted commit token waits between apply and commit (D18).
//!
//! A seam, so a store can be added when a commit on another carrier needs one. The only
//! implementation keeps the token in memory for one invocation: zeroed on drop, never shown
//! by `Debug` or `Display` (it implements neither), never logged, written or recorded.

use radio_hand::control::COMMIT_TOKEN_LEN;
use zeroize::{Zeroize, Zeroizing};

/// Holds at most one commit token.
pub trait CommitTokenHolder {
    /// Take the token, zeroing the caller's copy.
    fn keep(&mut self, token: &mut [u8; COMMIT_TOKEN_LEN]);
    /// Hand the token out for the commit; the holder no longer has it.
    fn release(&mut self) -> Option<Zeroizing<[u8; COMMIT_TOKEN_LEN]>>;
    /// Forget the token without using it (revert, expiry, or failure).
    fn discard(&mut self);
}

/// The token in this process's memory only, for the life of one invocation.
#[derive(Default)]
pub struct InMemoryTokenHolder {
    token: Option<Zeroizing<[u8; COMMIT_TOKEN_LEN]>>,
}

impl InMemoryTokenHolder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a token is held; the only fact about it this type will tell.
    pub fn holds(&self) -> bool {
        self.token.is_some()
    }
}

impl CommitTokenHolder for InMemoryTokenHolder {
    fn keep(&mut self, token: &mut [u8; COMMIT_TOKEN_LEN]) {
        self.token = Some(Zeroizing::new(*token));
        token.zeroize();
    }

    fn release(&mut self) -> Option<Zeroizing<[u8; COMMIT_TOKEN_LEN]>> {
        self.token.take()
    }

    fn discard(&mut self) {
        // Dropping the `Zeroizing` wrapper zeroes the bytes.
        self.token = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeping_zeroes_the_source_and_release_hands_it_out_once() {
        let mut source = [0xA7; COMMIT_TOKEN_LEN];
        let mut holder = InMemoryTokenHolder::new();
        holder.keep(&mut source);
        assert_eq!(source, [0; COMMIT_TOKEN_LEN]);
        assert!(holder.holds());
        assert_eq!(*holder.release().unwrap(), [0xA7; COMMIT_TOKEN_LEN]);
        assert!(holder.release().is_none());
        holder.keep(&mut [1; COMMIT_TOKEN_LEN]);
        holder.discard();
        assert!(!holder.holds());
    }
}
