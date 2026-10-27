use std::time::Duration;

use lgh_core::LghError;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt;

use crate::process::ManagedProcess;

async fn wait_for_xconnect(display: &str) -> Result<(), LghError> {
    for _ in 0..50 {
        if let Ok((conn, _)) = RustConnection::connect(Some(display)) {
            let _ = conn.sync();
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(LghError::DisplayUnavailable(format!(
        "X server did not accept connections on {}",
        display
    )))
}

pub struct XvfbBackend;

impl XvfbBackend {
    pub async fn start(
        display: &str,
        width: u32,
        height: u32,
        depth: u32,
    ) -> Result<ManagedProcess, LghError> {
        let screen = format!("{}x{}x{}", width, height, depth);
        let proc = ManagedProcess::spawn(
            "Xvfb",
            &[display, "-screen", "0", &screen, "-ac", "-noreset"],
            &[("DISPLAY", display)],
        )
        .await?;
        wait_for_xconnect(display).await?;
        Ok(proc)
    }
}
