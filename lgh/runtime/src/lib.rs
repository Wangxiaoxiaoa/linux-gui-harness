#![forbid(unsafe_code)]
use std::sync::Arc;

use std::sync::atomic::{AtomicU32, Ordering};

use lgh_core::LghError;
use uuid::Uuid;

pub mod clipboard;
pub mod compositor;
pub mod display;
pub mod process;
pub mod wm;
pub mod xserver;

pub use display::{Backend, Display, DisplayConfig, DisplayKind};
pub use process::ManagedProcess;

/// Ensure dynamically-loaded runtime binaries (sway, wtype, grim, x11vnc,
/// Xvfb, ...) can resolve their private libraries even when the daemon was
/// started without the runtime environment sourced: derive the lib dir from
/// the resolved `sway` binary (`<prefix>/usr/bin/sway` →
/// `<prefix>/usr/lib/x86_64-linux-gnu`) and prepend it to `LD_LIBRARY_PATH`
/// in the current process, so every spawned child inherits it.
pub fn ensure_runtime_libs() {
    let lib_dir = which("sway")
        .and_then(|p| p.ancestors().nth(2).map(|d| d.join("lib/x86_64-linux-gnu")));
    let Some(lib_dir) = lib_dir else { return };
    if !lib_dir.is_dir() {
        return;
    }
    let cur = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
    let parts: Vec<_> = std::env::split_paths(&cur).collect();
    // The runtime ships BLAS in a flavor subdir; wayvnc needs libblas.so.3
    // from openblas-openmp at spawn time.
    let openblas = lib_dir.join("openblas-openmp");
    let mut add = vec![lib_dir];
    if openblas.is_dir() {
        add.push(openblas);
    }
    let add: Vec<_> = add
        .into_iter()
        .filter(|p| !parts.iter().any(|e| e == p))
        .collect();
    if add.is_empty() {
        return;
    }
    let next = std::env::join_paths(add.into_iter().chain(parts.into_iter()))
        .expect("join LD_LIBRARY_PATH");
    std::env::set_var("LD_LIBRARY_PATH", next);
}

/// Resolve a binary name via `PATH`.
fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

fn process_scoped_display_start() -> u32 {
    // Use the process id so that concurrent daemon instances do not collide
    // on the low display numbers starting at :99. Two constraints: display
    // numbers must stay below 65536, and x11rb computes the TCP port as
    // 6000 + display in u16, so keep 6000 + display < 65536
    // (i.e. display <= 59535, hence the 59_400 range from base 100).
    let pid = std::process::id();
    100 + (pid % 59_400)
}

pub struct Runtime {
    next_display: AtomicU32,
    clipboard: Arc<crate::clipboard::ClipboardHub>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            next_display: AtomicU32::new(process_scoped_display_start()),
            clipboard: Arc::new(crate::clipboard::ClipboardHub::new()),
        }
    }

    /// The shared clipboard hub (one per process).
    pub fn clipboard(&self) -> &Arc<crate::clipboard::ClipboardHub> {
        &self.clipboard
    }

    /// Start the desktop spoke (the user's `$DISPLAY`); called lazily so
    /// headless hosts skip it.
    fn ensure_desktop_spoke(&self) {
        let desktop = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
        self.clipboard.start_desktop_spoke(&desktop);
    }

    pub async fn create_display(&self, config: DisplayConfig) -> Result<Display, LghError> {
        let id = format!("d-{}", Uuid::new_v4().simple());
        let display = match config.backend {
            // The :N display number is an X11 concept; Wayland worlds get
            // their socket name probed from the compositor itself.
            Backend::Xvfb => {
                let num = self.next_display.fetch_add(1, Ordering::Relaxed);
                let display = format!(":{}", num);
                Display::create_xvfb(id, display, config).await?
            }
            Backend::SwayHeadless | Backend::SwayNested => {
                Display::create_wayland(id, config).await?
            }
        };
        // SDK-created displays join the shared clipboard by default.
        self.ensure_desktop_spoke();
        self.clipboard.start_spoke(display.display());
        Ok(display)
    }

    pub async fn attach_display(&self, display: &str) -> Result<Display, LghError> {
        // x11rb computes the TCP port as 6000 + display in u16; a display
        // number above 59535 overflows there (panic in debug, silent wrap in
        // release), so reject it up front with a clear error.
        let num: u32 = display
            .strip_prefix(':')
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| LghError::InvalidArgument(format!("invalid display: {display}")))?;
        if num > 59_535 {
            return Err(LghError::InvalidArgument(format!(
                "display number {num} exceeds the maximum 59535"
            )));
        }
        self.ensure_desktop_spoke();
        let id = display.to_string();
        Display::attach(id, display.to_string()).await
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use lgh_core::{CaptureDriver, InputDriver, MouseButton};

    #[tokio::test]
    async fn sdk_driver_access() {
        let runtime = Runtime::new();
        let mut display = runtime
            .create_display(DisplayConfig {
                width: 400,
                height: 300,
                depth: 24,
                backend: Default::default(),
            })
            .await
            .expect("create display");

        let driver = display.driver();
        driver.move_mouse(200, 150).await.expect("move");
        let (x, y) = driver.get_cursor_position().await.expect("cursor");
        assert_eq!((x, y), (200, 150));
        let shot = driver.screenshot().await.expect("screenshot");
        assert!(!shot.data.is_empty());
        driver
            .click(x, y, MouseButton::Left, 1)
            .await
            .expect("click");

        display.destroy().await.expect("destroy");
    }
}
