use std::path::PathBuf;
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use crate::types::{IpcRequest, IpcResponse};

pub struct IpcClient {
    socket_path: PathBuf,
}

impl IpcClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub async fn send(&self, req: &IpcRequest) -> Result<IpcResponse> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to yamusic-cli daemon at {}. Is daemon running? (Run 'yamusic-cli daemon' to start)",
                    self.socket_path.display()
                )
            })?;

        let req_json = serde_json::to_string(req)?;
        stream.write_all(req_json.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        let resp: IpcResponse = serde_json::from_str(&response_line)
            .with_context(|| format!("Invalid response from daemon: {}", response_line))?;

        Ok(resp)
    }

    pub fn is_running(&self) -> bool {
        if !self.socket_path.exists() {
            return false;
        }
        std::os::unix::net::UnixStream::connect(&self.socket_path).is_ok()
    }
}
