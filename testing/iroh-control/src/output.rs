//! Reserve a new receipt without replacing existing evidence.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub fn reserve(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::reserve;
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn existing_receipt_is_unchanged_and_new_receipt_is_writable() {
        let name = format!(
            "murm-iroh-output-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(name);
        fs::create_dir(&dir).unwrap();
        let existing = dir.join("existing.json");
        let fresh = dir.join("fresh.json");
        fs::write(&existing, b"sentinel\r\n").unwrap();
        assert_eq!(
            reserve(&existing).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&existing).unwrap(), b"sentinel\r\n");
        let mut output = reserve(&fresh).unwrap();
        output.write_all(b"new receipt").unwrap();
        drop(output);
        assert_eq!(fs::read(&fresh).unwrap(), b"new receipt");
        fs::remove_file(existing).unwrap();
        fs::remove_file(fresh).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
