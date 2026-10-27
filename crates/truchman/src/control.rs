//! One bridge connection in its line phase, over tokio: [`Command`]s out, [`LineBuf`] lines
//! in, and the leftover bytes handed to the data phase.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::proto::{Command, LineBuf, expect_ok};
use crate::reply::Reply;
use crate::stream::Stream;
use crate::{Error, Result};

pub(crate) struct Control {
    tcp: TcpStream,
    lines: LineBuf,
}

impl Control {
    /// Connect and complete `HELLO`.
    pub(crate) async fn open(addr: std::net::SocketAddr) -> Result<Self> {
        let tcp = TcpStream::connect(addr).await?;
        let _ = tcp.set_nodelay(true);
        let mut control = Self {
            tcp,
            lines: LineBuf::new(),
        };
        control.call(Command::Hello, "HELLO", "REPLY").await?;
        Ok(control)
    }

    pub(crate) async fn send(&mut self, command: Command<'_>) -> Result<()> {
        self.tcp.write_all(command.line()?.as_bytes()).await?;
        Ok(())
    }

    pub(crate) async fn read_line(&mut self) -> Result<String> {
        read_line(&mut self.tcp, &mut self.lines).await
    }

    /// Send `command`; its reply must be `topic op` with `RESULT=OK`.
    pub(crate) async fn call(
        &mut self,
        command: Command<'_>,
        topic: &str,
        op: &str,
    ) -> Result<Reply> {
        self.send(command).await?;
        expect_ok(&self.read_line().await?, topic, op)
    }

    /// The connection as a data stream, starting with any bytes already read past the reply.
    pub(crate) fn into_stream(self) -> Stream {
        Stream::new(self.tcp, self.lines.into_rest())
    }

    /// Split into a line reader and a writer, for a session's control connection, which
    /// reads replies and writes commands concurrently.
    pub(crate) fn into_split(self) -> (ControlReader, OwnedWriteHalf) {
        let (read, write) = self.tcp.into_split();
        (
            ControlReader {
                read,
                lines: self.lines,
            },
            write,
        )
    }
}

/// The reading half of a split [`Control`].
pub(crate) struct ControlReader {
    read: OwnedReadHalf,
    lines: LineBuf,
}

impl ControlReader {
    /// Cancel-safe: bytes read are kept in the line buffer across a dropped future.
    pub(crate) async fn read_line(&mut self) -> Result<String> {
        read_line(&mut self.read, &mut self.lines).await
    }
}

async fn read_line<R: AsyncRead + Unpin>(read: &mut R, lines: &mut LineBuf) -> Result<String> {
    loop {
        if let Some(line) = lines.pop()? {
            return Ok(line);
        }
        let mut chunk = [0u8; 2048];
        let n = read.read(&mut chunk).await?;
        if n == 0 {
            return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        lines.push(&chunk[..n]);
    }
}
