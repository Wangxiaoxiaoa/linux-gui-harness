use lgh_core::{DisplayInfo, LghError};
use lgh_driver::DefaultDriver;
use std::sync::Arc;

use crate::process::ManagedProcess;
use crate::wm::OpenboxWM;
use crate::xserver::XvfbBackend;

#[derive(Clone)]
pub enum DisplayKind {
    Harness,
    External,
}

/// Which display-service stack a sandbox world runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    /// Xvfb + openbox (X11 protocol). The original sandbox.
    #[default]
    Xvfb,
    /// sway headless (Wayland protocol, invisible).
    SwayHeadless,
    /// sway wayland backend (Wayland protocol, native window on the host).
    SwayNested,
}

#[derive(Clone)]
pub struct DisplayConfig {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub backend: Backend,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            depth: 24,
            backend: Backend::Xvfb,
        }
    }
}

pub struct Display {
    id: String,
    display: String,
    kind: DisplayKind,
    backend: Backend,
    width: u32,
    height: u32,
    compositor: Option<ManagedProcess>,
    wm: Option<ManagedProcess>,
    apps: std::sync::Mutex<Vec<ManagedProcess>>,
    driver: Arc<DefaultDriver>,
    /// Identity of the session that created this display, set by the
    /// daemon right after creation. `None` for attached external displays.
    owner: std::sync::Mutex<Option<String>>,
}

impl Display {
    pub async fn create_xvfb(
        id: String,
        display: String,
        config: DisplayConfig,
    ) -> Result<Self, LghError> {
        let compositor =
            XvfbBackend::start(&display, config.width, config.height, config.depth).await?;

        let wm = OpenboxWM::start(&display).await?;

        let driver = Arc::new(DefaultDriver::new(&display)?);

        Ok(Self {
            id,
            display: display.clone(),
            kind: DisplayKind::Harness,
            backend: config.backend,
            width: config.width,
            height: config.height,
            compositor: Some(compositor),
            wm: Some(wm),
            apps: std::sync::Mutex::new(Vec::new()),
            driver,
            owner: std::sync::Mutex::new(None),
        })
    }

    /// Wayland 世界：sway 组合器 + 沙盒内应用（S3 实现 sway 编排）。
    pub async fn create_wayland(
        id: String,
        config: DisplayConfig,
    ) -> Result<Self, LghError> {
        Err(LghError::NotSupported)
    }

    pub async fn attach(id: String, display: String) -> Result<Self, LghError> {
        let driver = Arc::new(DefaultDriver::new(&display)?);

        Ok(Self {
            id,
            display: display.clone(),
            kind: DisplayKind::External,
            backend: Backend::Xvfb,
            width: 0,
            height: 0,
            compositor: None,
            wm: None,
            apps: std::sync::Mutex::new(Vec::new()),
            driver,
            owner: std::sync::Mutex::new(None),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn kind(&self) -> &DisplayKind {
        &self.kind
    }

    /// Identity of the owning session, set by the daemon after creation.
    pub fn owner(&self) -> Option<String> {
        self.owner.lock().unwrap().clone()
    }

    pub fn set_owner(&self, owner: String) {
        *self.owner.lock().unwrap() = Some(owner);
    }

    pub fn is_external(&self) -> bool {
        matches!(self.kind, DisplayKind::External)
    }

    pub async fn launch_app(&self, command: &str, args: &[&str]) -> Result<u32, LghError> {
        let env: &[(&str, &str)] = match self.backend {
            Backend::Xvfb => &[("DISPLAY", &self.display)],
            // Wayland 世界：主显示是沙盒自己的 socket；沙盒内 XWayland
            // 服务 X11 应用（:M 为探测到的沙盒显示号）。
            _ => &[("WAYLAND_DISPLAY", &self.display)],
        };
        let proc = ManagedProcess::spawn(command, args, env).await?;
        let pid = proc.pid();
        self.apps.lock().unwrap().push(proc);
        Ok(pid)
    }

    pub async fn terminate_app(&self, pid: u32) -> Result<(), LghError> {
        let mut proc = {
            let mut apps = self.apps.lock().unwrap();
            let pos = apps
                .iter()
                .position(|p| p.pid() == pid)
                .ok_or_else(|| LghError::InvalidArgument(format!("pid {} not found", pid)))?;
            apps.remove(pos)
        };
        proc.kill().await
    }

    /// Access the automation driver for this display. Provides the full
    /// SDK surface: click, type, screenshot, AT-SPI, window management,
    /// clipboard, etc.
    pub fn driver(&self) -> &DefaultDriver {
        &self.driver
    }

    pub fn info(&self) -> DisplayInfo {
        DisplayInfo {
            display: self.display.clone(),
            width: self.width,
            height: self.height,
            app_count: self.apps.lock().unwrap().len(),
        }
    }

    pub async fn destroy(&mut self) -> Result<(), LghError> {
        if self.is_external() {
            return Err(LghError::InvalidArgument(
                "cannot destroy external display".into(),
            ));
        }
        let mut apps = {
            let mut apps = self.apps.lock().unwrap();
            std::mem::take(&mut *apps)
        };
        for app in apps.iter_mut() {
            let _ = app.kill().await;
        }
        if let Some(ref mut wm) = self.wm {
            let _ = wm.kill().await;
        }
        if let Some(ref mut compositor) = self.compositor {
            let _ = compositor.kill().await;
        }
        Ok(())
    }
}
