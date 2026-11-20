//! Where the board-minted commit token waits between apply and commit (D18).
//!
//! A seam, so a store can be added when a commit on another carrier needs one. The only
//! implementation keeps the token in memory for one invocation. The token itself is
//! postilion's [`CommitToken`] (D21): zeroed on drop, never `Copy` or `Clone`, its `Debug`
//! redacted; it is never logged, written or recorded.

use postilion::control::verified::CommitToken;

/// Holds at most one commit token.
pub trait CommitTokenHolder {
    /// Take the token; the caller no longer has it.
    fn keep(&mut self, token: CommitToken);
    /// Hand the token out for the commit; the holder no longer has it.
    fn release(&mut self) -> Option<CommitToken>;
    /// Forget the token without using it (revert, expiry, or failure).
    fn discard(&mut self);
}

/// The token in this process's memory only, for the life of one invocation.
#[derive(Default)]
pub struct InMemoryTokenHolder {
    token: Option<CommitToken>,
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
    fn keep(&mut self, token: CommitToken) {
        self.token = Some(token);
    }

    fn release(&mut self) -> Option<CommitToken> {
        self.token.take()
    }

    fn discard(&mut self) {
        // Dropping the token zeroes its bytes.
        self.token = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radio_hand::control::COMMIT_TOKEN_LEN;

    #[test]
    fn keeping_zeroes_the_source_and_release_hands_it_out_once() {
        let mut source = [0xA7; COMMIT_TOKEN_LEN];
        let mut holder = InMemoryTokenHolder::new();
        holder.keep(CommitToken::take_from(&mut source));
        assert_eq!(source, [0; COMMIT_TOKEN_LEN]);
        assert!(holder.holds());
        assert_eq!(
            holder.release().unwrap().expose(),
            &[0xA7; COMMIT_TOKEN_LEN]
        );
        assert!(holder.release().is_none());
        holder.keep(CommitToken::take_from(&mut [1; COMMIT_TOKEN_LEN]));
        holder.discard();
        assert!(!holder.holds());
    }
}
