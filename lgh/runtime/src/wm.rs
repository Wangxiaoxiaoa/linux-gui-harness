use lgh_core::LghError;

use crate::process::ManagedProcess;

pub struct OpenboxWM;

impl OpenboxWM {
    pub async fn start(display: &str) -> Result<ManagedProcess, LghError> {
        ManagedProcess::spawn("openbox", &["--replace"], &[("DISPLAY", display)]).await
    }
}
