//! One bridge connection in its line phase, over tokio: [`Command`]s out, [`LineBuf`] lines
//! in, and the leftover bytes handed to the data phase.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
        loop {
            if let Some(line) = self.lines.pop()? {
                return Ok(line);
            }
            let mut chunk = [0u8; 2048];
            let n = self.tcp.read(&mut chunk).await?;
            if n == 0 {
                return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            self.lines.push(&chunk[..n]);
        }
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
}
