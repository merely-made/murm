use super::*;

fn scope() -> CompanionScope {
    CompanionScope::Meshtastic {
        realm: "test-household".into(),
        node: 7,
        channel: 8,
    }
}

struct FailingJournal;
impl RadioJournal for FailingJournal {
    fn load(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn commit(&mut self, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("disk refused"))
    }
}
#[test]
fn failing_durable_owner_cannot_publish_a_mailbox() {
    assert!(RadioComms::open_companion(scope(), FailingJournal).is_err());
}

struct FailOnceJournal {
    commits: usize,
}
impl RadioJournal for FailOnceJournal {
    fn load(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn commit(&mut self, _: &[u8]) -> std::io::Result<()> {
        self.commits += 1;
        if self.commits == 2 {
            Err(std::io::Error::other("disk refused queued intent"))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn failed_intent_commit_never_publishes_and_file_owner_is_exclusive() {
    let mailbox = RadioComms::open_companion(scope(), FailOnceJournal { commits: 0 }).unwrap();
    let conversation = mailbox
        .conversation(ProtocolKind::Meshtastic, "00000009")
        .unwrap();
    let mut draft = Draft::reply_to(conversation);
    draft.body = "must persist first".into();
    assert!(mailbox.model().send(&draft).await.is_err());
    assert!(mailbox.records().unwrap().is_empty());
    assert_eq!(mailbox.model().send(&draft).await.unwrap().0, "out-1");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("exclusive.json");
    let first = RadioComms::open_companion(scope(), FileJournal::new(&path)).unwrap();
    assert!(RadioComms::open_companion(scope(), FileJournal::new(&path)).is_err());
    drop(first);
    assert!(RadioComms::reopen(FileJournal::new(&path)).is_ok());
}
