//! Stdio transport — newline-delimited JSON over child stdin/stdout.

use anyhow::{Context, Result, bail};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

const TIMEOUT: Duration = Duration::from_secs(30);

pub struct StdioTransport {
    pub child: Child,
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
}

impl StdioTransport {
    pub fn new(mut command: tokio::process::Command) -> Result<Self> {
        use std::process::Stdio;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().context("failed to spawn MCP server")?;
        let stdout = child.stdout.take().context("missing stdout")?;
        let stdin = child.stdin.take().context("missing stdin")?;
        Ok(Self {
            child,
            reader: BufReader::new(stdout),
            writer: stdin,
        })
    }

    pub async fn request(&mut self, msg: serde_json::Value) -> Result<serde_json::Value> {
        let mut buf = serde_json::to_vec(&msg)?;
        buf.push(b'\n');
        self.writer
            .write_all(&buf)
            .await
            .context("write to MCP stdin")?;
        self.writer.flush().await?;

        // Read lines until we get valid JSON (skip non-JSON diagnostics).
        let mut line = String::new();
        let read_fut = async {
            loop {
                line.clear();
                let n = self
                    .reader
                    .read_line(&mut line)
                    .await
                    .context("read from MCP stdout")?;
                if n == 0 {
                    bail!("MCP server closed stdout");
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                    return Ok(v);
                }
            }
        };
        tokio::time::timeout(TIMEOUT, read_fut)
            .await
            .context("MCP server response timed out")?
    }

    pub async fn notify(&mut self, msg: serde_json::Value) -> Result<()> {
        let mut buf = serde_json::to_vec(&msg)?;
        buf.push(b'\n');
        self.writer
            .write_all(&buf)
            .await
            .context("write to MCP stdin")?;
        self.writer.flush().await?;
        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
