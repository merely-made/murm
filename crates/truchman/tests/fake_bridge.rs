//! truchman against a scripted SAM bridge: each command line gets a canned answer, and a
//! connection can then turn into an echoing data stream.

use std::io;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use truchman::{Bridge, Destination, Error, Key, PrivateKey, SessionOptions, StreamSession};

enum Act {
    Line(String),
    Raw(Vec<u8>),
    Echo,
    Close,
}

type Script = Arc<dyn Fn(&str) -> Vec<Act> + Send + Sync>;

/// A bridge answering every `HELLO` with 3.1 and every other line by `script`. The lines it
/// received, `HELLO` excluded, land in the returned log.
async fn fake(
    script: impl Fn(&str) -> Vec<Act> + Send + Sync + 'static,
) -> (Bridge, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let script: Script = Arc::new(script);
    let seen = Arc::clone(&log);
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let (script, seen) = (Arc::clone(&script), Arc::clone(&seen));
            tokio::spawn(async move {
                let (read, mut write) = tcp.into_split();
                let mut read = BufReader::new(read);
                let mut line = String::new();
                loop {
                    line.clear();
                    if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    let acts = if line.starts_with("HELLO VERSION") {
                        vec![Act::Line("HELLO REPLY RESULT=OK VERSION=3.1".into())]
                    } else {
                        seen.lock().unwrap().push(line.to_string());
                        script(line)
                    };
                    for act in acts {
                        match act {
                            Act::Line(l) => {
                                write.write_all(format!("{l}\n").as_bytes()).await.unwrap()
                            }
                            Act::Raw(b) => write.write_all(&b).await.unwrap(),
                            Act::Close => return,
                            Act::Echo => {
                                let mut buf = [0u8; 1024];
                                loop {
                                    match read.read(&mut buf).await {
                                        Ok(0) | Err(_) => return,
                                        Ok(n) => write.write_all(&buf[..n]).await.unwrap(),
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    (Bridge::new(addr), log)
}

/// Destination and private-key base64 for a destination with a 4-byte key certificate.
fn keypair(seed: u8) -> (String, String) {
    let mut public: Vec<u8> = (0..384).map(|i| (i as u8).wrapping_mul(seed)).collect();
    public.extend([5, 0, 4, 0, 7, 0, 4]);
    let mut private = public.clone();
    private.extend([seed; 288]);
    (
        truchman::base64_encode(&public),
        truchman::base64_encode(&private),
    )
}

#[tokio::test]
async fn generates_destinations_and_resolves_names() {
    let (public, private) = keypair(3);
    let (p, s) = (public.clone(), private.clone());
    let (bridge, log) = fake(move |line| match line {
        "DEST GENERATE SIGNATURE_TYPE=7" => vec![Act::Line(format!("DEST REPLY PUB={p} PRIV={s}"))],
        "NAMING LOOKUP NAME=peer.i2p" => vec![Act::Line(format!(
            "NAMING REPLY RESULT=OK NAME=peer.i2p VALUE={p}"
        ))],
        _ => vec![Act::Line("NAMING REPLY RESULT=KEY_NOT_FOUND NAME=x".into())],
    })
    .await;
    let key = bridge.generate(truchman::SIGNATURE_ED25519).await.unwrap();
    let dest = Destination::from_base64(&public).unwrap();
    assert_eq!(key.destination(), &dest);
    assert_eq!(key.expose_base64(), private);
    assert_eq!(bridge.resolve("peer.i2p").await.unwrap(), dest);
    // A base64 destination needs no lookup.
    assert_eq!(bridge.resolve(&public).await.unwrap(), dest);
    let missing = bridge.lookup("nobody.i2p").await.unwrap_err();
    assert_eq!(missing.result(), Some("KEY_NOT_FOUND"));
    assert_eq!(log.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_transient_session_connects_and_keeps_bytes_sent_with_the_status() {
    let (public, private) = keypair(5);
    let s = private.clone();
    let (bridge, log) = fake(move |line| {
        if line.starts_with("SESSION CREATE") {
            vec![Act::Line(format!(
                "SESSION STATUS RESULT=OK DESTINATION={s}"
            ))]
        } else if line.starts_with("STREAM CONNECT") {
            // Data arriving in the same segment as the status must not be lost.
            vec![
                Act::Raw(b"STREAM STATUS RESULT=OK\nearly".to_vec()),
                Act::Echo,
            ]
        } else {
            vec![Act::Close]
        }
    })
    .await;
    let options = SessionOptions {
        options: vec![("inbound.length".into(), "1".into())],
        ..SessionOptions::default()
    };
    let session = StreamSession::create(bridge, Key::Transient, options)
        .await
        .unwrap();
    assert_eq!(session.private_key().expose_base64(), private);
    let peer = Destination::from_base64(&public).unwrap();
    let mut stream = session.connect(&peer).await.unwrap();
    let mut early = [0u8; 5];
    stream.read_exact(&mut early).await.unwrap();
    assert_eq!(&early, b"early");
    let (mut read, mut write) = stream.into_split();
    write.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    read.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"ping");

    let log = log.lock().unwrap();
    let create = &log[0];
    assert!(create.starts_with(&format!("SESSION CREATE STYLE=STREAM ID={} ", session.id())));
    assert!(create.ends_with("DESTINATION=TRANSIENT SIGNATURE_TYPE=7 inbound.length=1"));
    assert_eq!(
        log[1],
        format!(
            "STREAM CONNECT ID={} DESTINATION={public} SILENT=false",
            session.id()
        )
    );
}

#[tokio::test]
async fn accept_returns_the_peer_and_its_first_bytes() {
    let (mine, my_private) = keypair(7);
    let (theirs, _) = keypair(9);
    let t = theirs.clone();
    let (bridge, log) = fake(move |line| {
        if line.starts_with("SESSION CREATE") {
            vec![Act::Line(
                "SESSION STATUS RESULT=OK DESTINATION=ignored".into(),
            )]
        } else if line.starts_with("STREAM ACCEPT") {
            vec![
                Act::Line("STREAM STATUS RESULT=OK".into()),
                Act::Raw(format!("{t} FROM_PORT=0 TO_PORT=0\nhello").into_bytes()),
                Act::Echo,
            ]
        } else {
            vec![Act::Close]
        }
    })
    .await;
    let key = PrivateKey::from_base64(&my_private).unwrap();
    let options = SessionOptions {
        nickname: Some("rns".into()),
        ..SessionOptions::default()
    };
    let session = StreamSession::create(bridge, Key::Persistent(key), options)
        .await
        .unwrap();
    assert_eq!(
        session.destination(),
        &Destination::from_base64(&mine).unwrap()
    );
    let (mut stream, peer) = session.accept().await.unwrap();
    assert_eq!(peer, Destination::from_base64(&theirs).unwrap());
    let mut hello = [0u8; 5];
    stream.read_exact(&mut hello).await.unwrap();
    assert_eq!(&hello, b"hello");

    let log = log.lock().unwrap();
    assert_eq!(
        log[0],
        format!("SESSION CREATE STYLE=STREAM ID=rns DESTINATION={my_private}")
    );
    assert_eq!(log[1], "STREAM ACCEPT ID=rns SILENT=false");
}

#[tokio::test]
async fn refusals_carry_the_bridge_result() {
    let (public, private) = keypair(11);
    let (bridge, _) = fake(move |line| {
        if line.contains("ID=dup") {
            vec![Act::Line("SESSION STATUS RESULT=DUPLICATED_DEST".into())]
        } else if line.starts_with("SESSION CREATE") {
            vec![Act::Line("SESSION STATUS RESULT=OK DESTINATION=x".into())]
        } else {
            vec![Act::Line(
                r#"STREAM STATUS RESULT=CANT_REACH_PEER MESSAGE="no lease set""#.into(),
            )]
        }
    })
    .await;
    let key = PrivateKey::from_base64(&private).unwrap();
    let dup = SessionOptions {
        nickname: Some("dup".into()),
        ..SessionOptions::default()
    };
    let err = StreamSession::create(bridge, Key::Persistent(key.clone()), dup)
        .await
        .unwrap_err();
    assert_eq!(err.result(), Some("DUPLICATED_DEST"));

    let session = StreamSession::create(bridge, Key::Persistent(key), SessionOptions::default())
        .await
        .unwrap();
    let err = session
        .connect(&Destination::from_base64(&public).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(&err, Error::Refused { message: Some(m), .. } if m == "no lease set"));
    assert_eq!(io::Error::from(err).kind(), io::ErrorKind::HostUnreachable);
}

#[tokio::test]
async fn a_bridge_without_a_common_version_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge = Bridge::new(listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 128];
        let _ = tcp.read(&mut buf).await;
        tcp.write_all(b"HELLO REPLY RESULT=NOVERSION\n")
            .await
            .unwrap();
    });
    let err = bridge.lookup("x.i2p").await.unwrap_err();
    assert_eq!(err.result(), Some("NOVERSION"));
}

#[tokio::test]
async fn the_session_answers_pings_and_notices_the_bridge_closing() {
    let (_, private) = keypair(13);
    let (bridge, log) = fake(move |line| {
        if line.starts_with("SESSION CREATE") {
            vec![
                Act::Line(format!("SESSION STATUS RESULT=OK DESTINATION={private}")),
                Act::Line("PING 42".into()),
            ]
        } else {
            // The PONG: now drop the session.
            vec![Act::Close]
        }
    })
    .await;
    let session = StreamSession::create(bridge, Key::Transient, SessionOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), session.closed())
        .await
        .expect("session close noticed");
    assert!(!session.is_alive());
    assert_eq!(log.lock().unwrap()[1], "PONG 42");
}

#[tokio::test]
async fn values_that_would_break_the_line_never_reach_the_bridge() {
    let (bridge, log) = fake(|_| vec![Act::Close]).await;
    let smuggled = SessionOptions {
        options: vec![("inbound.length".into(), "1\nSTREAM CONNECT".into())],
        ..SessionOptions::default()
    };
    let err = StreamSession::create(bridge, Key::Transient, smuggled)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    let spaced = SessionOptions {
        nickname: Some("a b".into()),
        ..SessionOptions::default()
    };
    assert!(
        StreamSession::create(bridge, Key::Transient, spaced)
            .await
            .is_err()
    );
    let keyed = SessionOptions {
        options: vec![("a=b".into(), "1".into())],
        ..SessionOptions::default()
    };
    assert!(
        StreamSession::create(bridge, Key::Transient, keyed)
            .await
            .is_err()
    );
    assert!(bridge.lookup("x.i2p NAME=y").await.is_err());
    assert!(log.lock().unwrap().is_empty());
    // Positive control: a clean command on the same bridge does reach it.
    let _ = bridge.lookup("x.i2p").await;
    assert_eq!(*log.lock().unwrap(), ["NAMING LOOKUP NAME=x.i2p"]);
}

#[tokio::test]
async fn a_session_looks_names_up_on_its_own_connection() {
    let (public, private) = keypair(15);
    // A bridge that accepts exactly one connection, the session's, then stops listening:
    // a lookup that opened a fresh connection would be refused, not answered.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge = Bridge::new(listener.local_addr().unwrap());
    let (p, s) = (public.clone(), private.clone());
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        drop(listener);
        let (read, mut write) = tcp.into_split();
        let mut read = BufReader::new(read);
        let mut line = String::new();
        while read.read_line(&mut line).await.unwrap_or(0) > 0 {
            let reply = match line.trim_end() {
                l if l.starts_with("HELLO") => "HELLO REPLY RESULT=OK VERSION=3.1".to_string(),
                l if l.starts_with("SESSION CREATE") => {
                    format!("SESSION STATUS RESULT=OK DESTINATION={s}")
                }
                "NAMING LOOKUP NAME=peer.b32.i2p" => {
                    format!("NAMING REPLY RESULT=OK NAME=peer.b32.i2p VALUE={p}")
                }
                l => format!(
                    "NAMING REPLY RESULT=KEY_NOT_FOUND NAME={}",
                    l.rsplit('=').next().unwrap()
                ),
            };
            write
                .write_all(
                    format!(
                        "{reply}
"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            line.clear();
        }
    });
    let session = StreamSession::create(bridge, Key::Transient, SessionOptions::default())
        .await
        .unwrap();
    let dest = Destination::from_base64(&public).unwrap();
    assert_eq!(session.lookup("peer.b32.i2p").await.unwrap(), dest);
    assert_eq!(session.resolve("peer.b32.i2p").await.unwrap(), dest);
    assert_eq!(
        session.resolve(&public).await.unwrap(),
        dest,
        "base64 needs no lookup"
    );
    let missing = session.lookup("gone.b32.i2p").await.unwrap_err();
    assert_eq!(missing.result(), Some("KEY_NOT_FOUND"));
    assert!(session.lookup("a b").await.is_err());
    // Positive control: a fresh-connection lookup on the same bridge is refused, so the
    // answers above can only have come over the session's connection.
    assert!(matches!(
        bridge.lookup("peer.b32.i2p").await,
        Err(Error::Io(_))
    ));
    assert!(session.is_alive());
}

#[tokio::test]
async fn a_lookup_fails_cleanly_once_the_session_is_gone() {
    let (_, private) = keypair(17);
    let (bridge, _) = fake(move |line| {
        if line.starts_with("SESSION CREATE") {
            vec![Act::Line(format!(
                "SESSION STATUS RESULT=OK DESTINATION={private}"
            ))]
        } else {
            // The lookup: drop the session instead of answering.
            vec![Act::Close]
        }
    })
    .await;
    let session = StreamSession::create(bridge, Key::Transient, SessionOptions::default())
        .await
        .unwrap();
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.lookup("peer.b32.i2p"),
    )
    .await
    .expect("the pending lookup is answered when the session closes")
    .unwrap_err();
    assert!(matches!(err, Error::Io(ref e) if e.kind() == io::ErrorKind::NotConnected));
    session.closed().await;
    assert!(session.lookup("again.b32.i2p").await.is_err());
}
