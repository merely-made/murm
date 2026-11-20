//! Whether an unattended loop may still try a host write.
//!
//! A board whose transport cannot tell that a host has left bounds each write; one that runs
//! out closes the gate, and only bytes from the host open it again. A departed host then costs
//! one bounded write per batch of host input rather than one per reply.

/// Open until a bounded write runs out; open again when the host is heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteGate {
    open: bool,
}

impl WriteGate {
    pub const fn open() -> Self {
        Self { open: true }
    }

    pub const fn is_open(self) -> bool {
        self.open
    }

    /// A read returned `length` bytes. Only a non-empty read proves someone is there.
    pub fn heard(&mut self, length: usize) {
        if length > 0 {
            self.open = true;
        }
    }

    /// A write ran out of time.
    pub fn expired(&mut self) {
        self.open = false;
    }
}

impl Default for WriteGate {
    fn default() -> Self {
        Self::open()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_open_and_an_expired_write_closes_it() {
        let mut gate = WriteGate::open();
        assert!(gate.is_open());
        gate.expired();
        assert!(!gate.is_open());
    }

    #[test]
    fn an_empty_read_does_not_reopen_it() {
        let mut gate = WriteGate::open();
        gate.expired();
        gate.heard(0);
        assert!(!gate.is_open());
    }

    #[test]
    fn host_bytes_reopen_it() {
        let mut gate = WriteGate::open();
        gate.expired();
        gate.heard(5);
        assert!(gate.is_open());
        gate.heard(3);
        assert!(gate.is_open());
    }
}
