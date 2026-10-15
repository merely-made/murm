//! An RNode reached over TCP (`RNodeInterface.py` 1452-1582).

use std::io;
use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

/// `TCPConnection.CONNECT_TIMEOUT`.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// RNS's keepalive profile: probe after 5 s idle, every 2 s, 12 times; on Linux the
/// connection also fails once data stays unacknowledged for 24 s (1460-1463, 1515-1520).
const PROBE_AFTER: Duration = Duration::from_secs(5);
const PROBE_INTERVAL: Duration = Duration::from_secs(2);
const PROBES: u32 = 12;
#[cfg(any(target_os = "linux", target_os = "android"))]
const USER_TIMEOUT: Duration = Duration::from_secs(24);

/// Connect with `TCP_NODELAY` and the keepalive profile. Unlike RNS on macOS, which sets
/// only the idle time (1522-1526), the interval and count apply on every platform.
pub(super) async fn connect(host: &str, port: u16) -> io::Result<TcpStream> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "RNode TCP connect timed out"))??;
    stream.set_nodelay(true)?;
    let socket = SockRef::from(&stream);
    let keepalive = TcpKeepalive::new()
        .with_time(PROBE_AFTER)
        .with_interval(PROBE_INTERVAL)
        .with_retries(PROBES);
    socket.set_tcp_keepalive(&keepalive)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    socket.set_tcp_user_timeout(Some(USER_TIMEOUT))?;
    Ok(stream)
}
