//! Wayland sandbox driver — implements the same `lgh-core` driver traits as
//! the X11 `DefaultDriver`, but talks the Wayland protocol to a per-world
//! sway compositor.
//!
//! Input injection: zwlr_virtual_pointer_v1 (pointer) + wtype (keyboard).
//! Capture: grim (wlr-screencopy). Window ops: sway IPC.
//!
//! All external binary calls (wtype, grim, swaymsg) run as subprocesses
//! with the world's `WAYLAND_DISPLAY` / `SWAYSOCK` env — no persistent
//! connections except the virtual pointer session for drags.

pub mod pointer;

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use lgh_core::{
    A11yDriver, Bounds, CaptureDriver, Driver, GetWindowStateResult, InputDriver, LghError,
    MouseButton, RegionCapture, Screenshot, VerificationResult, WindowDriver,
};
use lgh_core::DesktopOverview;

pub struct WaylandDriver {
    /// The world's own Wayland socket name (e.g. `wayland-3`).
    display: String,
    /// The world's sway IPC socket path.
    ipc: PathBuf,
    /// The compositor output name (e.g. `HEADLESS-1`).
    output: String,
    width: u32,
    height: u32,
    cursor: Mutex<(u32, u32)>,
}

impl WaylandDriver {
    pub fn new(display: String, ipc: PathBuf, output: String, width: u32, height: u32) -> Self {
        Self {
            display,
            ipc,
            output,
            width,
            height,
            cursor: Mutex::new((0, 0)),
        }
    }

    fn swaymsg(&self, args: &[&str]) -> Result<String, LghError> {
        let mut cmd = std::process::Command::new("swaymsg");
        cmd.env("SWAYSOCK", &self.ipc).args(args);
        let out = cmd
            .output()
            .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "swaymsg {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

// ---- InputDriver ----

#[async_trait]
impl InputDriver for WaylandDriver {
    async fn click(&self, x: i32, y: i32, button: MouseButton, count: u32) -> Result<(), LghError> {
        let code = match button {
            MouseButton::Left => pointer::BTN_LEFT,
            MouseButton::Right => pointer::BTN_RIGHT,
            MouseButton::Middle => pointer::BTN_MIDDLE,
        };
        let mut s = pointer::VPtrSession::open(&self.display, self.width, self.height)
            .map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;
        s.move_abs(x as u32, y as u32)?;
        for _ in 0..count.max(1) {
            s.button(code, true)?;
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            s.button(code, false)?;
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
        s.close()?;
        *self.cursor.lock().unwrap() = (x as u32, y as u32);
        Ok(())
    }

    async fn move_mouse(&self, x: i32, y: i32) -> Result<(), LghError> {
        let mut s = pointer::VPtrSession::open(&self.display, self.width, self.height)
            .map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;
        s.move_abs(x as u32, y as u32)?;
        s.close()?;
        Ok(())
    }

    async fn scroll(&self, dx: i32, dy: i32) -> Result<(), LghError> {
        let mut s = pointer::VPtrSession::open(&self.display, self.width, self.height)
            .map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;
        s.scroll(dx as f64, dy as f64)?;
        s.close()?;
        Ok(())
    }

    async fn drag(
        &self,
        x1: i32, y1: i32, x2: i32, y2: i32,
        modifiers: &[&str],
    ) -> Result<(), LghError> {
        let code = pointer::BTN_LEFT;
        let steps = 25;
        let mut s = pointer::VPtrSession::open(&self.display, self.width, self.height)
            .map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;
        s.move_abs(x1 as u32, y1 as u32)?;
        s.button(code, true)?;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let x = x1 as f64 + (x2 as f64 - x1 as f64) * t;
            let y = y1 as f64 + (y2 as f64 - y1 as f64) * t;
            s.move_abs(x as u32, y as u32)?;
        }
        s.button(code, false)?;
        s.close()?;
        Ok(())
    }

    async fn type_text(&self, _pid: Option<u32>, text: &str) -> Result<(), LghError> {
        let mut cmd = tokio::process::Command::new("wtype");
        cmd.env("WAYLAND_DISPLAY", &self.display).arg("--").arg(text);
        let out = cmd.output().await.map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::ProcessSpawnFailed(format!(
                "wtype failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }

    async fn key(&self, key: &str, modifiers: &[&str]) -> Result<(), LghError> {
        let keysym = keysym_name(key)?;
        let mut args: Vec<String> = vec![];
        for m in modifiers {
            args.push("-M".into());
            args.push(modifier_name(m)?);
        }
        args.push("-k".into());
        args.push(keysym);
        for m in modifiers.iter().rev() {
            args.push("-m".into());
            args.push(modifier_name(m)?);
        }
        let mut cmd = tokio::process::Command::new("wtype");
        cmd.env("WAYLAND_DISPLAY", &self.display).args(&args);
        let out = cmd.output().await.map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::ProcessSpawnFailed(format!(
                "wtype failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }

    async fn get_cursor_position(&self) -> Result<(i32, i32), LghError> {
        let pos = *self.cursor.lock().unwrap();
        Ok((pos.0 as i32, pos.1 as i32))
    }
}

// ---- CaptureDriver ----

#[async_trait]
impl CaptureDriver for WaylandDriver {
    async fn screenshot(&self) -> Result<Screenshot, LghError> {
        let mut cmd = tokio::process::Command::new("grim");
        cmd.env("WAYLAND_DISPLAY", &self.display).arg("-");
        let out = cmd.output().await.map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "grim failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(Screenshot { data: out.stdout })
    }

    async fn screenshot_window(&self, _window_id: u32) -> Result<Screenshot, LghError> {
        // In a tiling compositor the window occupies a known region of the
        // output; a full-output capture is equivalent.
        self.screenshot().await
    }

    async fn capture_at_cursor(&self) -> Result<(u32, Screenshot), LghError> {
        let shot = self.screenshot().await?;
        let pos = *self.cursor.lock().unwrap();
        Ok((pos.0, shot))
    }

    async fn capture_region(
        &self, window_id: u32, x1: f64, y1: f64, x2: f64, y2: f64,
    ) -> Result<RegionCapture, LghError> {
        let _ = window_id;
        let dw = ((x2 - x1).abs()) as u32;
        let dh = ((y2 - y1).abs()) as u32;
        let mut cmd = tokio::process::Command::new("grim");
        cmd.env("WAYLAND_DISPLAY", &self.display)
            .args(["-g", &format!("{:.0} {:.0} {:.0} {:.0}", x1, y1, dw as f64, dh as f64), "-"]);
        let out = cmd.output().await.map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "grim region failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(RegionCapture {
            screenshot: Screenshot { data: out.stdout },
            display_x: x1 as i32,
            display_y: y1 as i32,
            scale: 1.0,
        })
    }
}

// ---- WindowDriver ----

#[async_trait]
impl WindowDriver for WaylandDriver {
    async fn focus_window(&self, window_id: u32) -> Result<(), LghError> {
        let out = std::process::Command::new("swaymsg")
            .env("SWAYSOCK", &self.ipc)
            .args(["[", &format!("con_id={window_id}"), "]", "focus"])
            .output()
            .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "focus failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }

    async fn set_window_frame(
        &self, window_id: u32, x: i32, y: i32, w: u32, h: u32,
    ) -> Result<(), LghError> {
        // Sway tiling layout treats geometry as a suggestion.
        let out = std::process::Command::new("swaymsg")
            .env("SWAYSOCK", &self.ipc)
            .args([
                "[", &format!("con_id={window_id}"), "]",
                "move", "absolute", "position", &x.to_string(), &y.to_string(),
                "resize", "set", &w.to_string(), "px", &h.to_string(), "px",
            ])
            .output()
            .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "set_frame failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }

    async fn close_window(&self, window_id: u32) -> Result<(), LghError> {
        let out = std::process::Command::new("swaymsg")
            .env("SWAYSOCK", &self.ipc)
            .args(["[", &format!("con_id={window_id}"), "]", "kill"])
            .output()
            .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;
        if !out.status.success() {
            return Err(LghError::DisplayUnavailable(format!(
                "close failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(())
    }
}

// ---- A11yDriver (minimal: NotSupported until AT-SPI validation) ----

#[async_trait]
impl A11yDriver for WaylandDriver {
    async fn get_window_state(
        &self, _pid: u32, _window_id: u32, _include_tree: bool, _include_screenshot: bool,
    ) -> Result<GetWindowStateResult, LghError> {
        Err(LghError::NotSupported)
    }

    async fn get_desktop_overview(&self) -> Result<DesktopOverview, LghError> {
        Err(LghError::NotSupported)
    }

    async fn set_value(
        &self, _pid: u32, _index: usize, _value: &str, _focus_allowed: bool,
    ) -> Result<(), LghError> {
        Err(LghError::NotSupported)
    }

    async fn element_frame(&self, _pid: u32, _index: usize) -> Result<Bounds, LghError> {
        Err(LghError::NotSupported)
    }

    async fn invoke_menu(&self, _pid: u32, _path: &[String]) -> Result<(), LghError> {
        Err(LghError::NotSupported)
    }

    async fn verify_state(
        &self, _pid: u32, _expect: &[lgh_core::StateExpectation],
    ) -> Result<VerificationResult, LghError> {
        Err(LghError::NotSupported)
    }
}

// ---- Driver (composite) ----

#[async_trait]
impl Driver for WaylandDriver {
    async fn click_element(
        &self, _pid: u32, _index: usize, _button: MouseButton,
    ) -> Result<(), LghError> {
        Err(LghError::NotSupported)
    }
}

// ---- helpers ----

fn keysym_name(key: &str) -> Result<String, LghError> {
    let k = key.to_ascii_lowercase();
    Ok(match k.as_str() {
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "space" => "space",
        "backspace" => "BackSpace",
        "del" | "delete" => "Delete",
        "ins" | "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pgup" | "pageup" => "Prior",
        "pgdn" | "pagedown" => "Next",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        other => {
            if other.len() == 1
                || (other.starts_with('f') && other[1..].parse::<u32>().is_ok())
            {
                other
            } else if other == "f" {
                return Err(LghError::InvalidArgument(format!("unknown key {key:?}")));
            } else {
                other
            }
        }
    }.to_string())
}

fn modifier_name(m: &str) -> Result<String, LghError> {
    Ok(match m.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "ctrl",
        "alt" => "alt",
        "shift" => "shift",
        "super" | "logo" | "win" | "meta" => "logo",
        other => return Err(LghError::InvalidArgument(format!("unknown modifier {other:?}"))),
    }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- keysym_name ----

    #[test]
    fn keysym_enter() {
        assert_eq!(keysym_name("enter").unwrap(), "Return");
        assert_eq!(keysym_name("Return").unwrap(), "Return");
    }
    #[test]
    fn keysym_escape() {
        assert_eq!(keysym_name("escape").unwrap(), "Escape");
        assert_eq!(keysym_name("esc").unwrap(), "Escape");
    }
    #[test]
    fn keysym_single_char() {
        assert_eq!(keysym_name("a").unwrap(), "a");
        assert_eq!(keysym_name("Z").unwrap(), "z"); // lowercased
    }
    #[test]
    fn keysym_f_key() {
        assert_eq!(keysym_name("f5").unwrap(), "f5");
    }
    #[test]
    fn keysym_unknown_multi_char() {
        // Multi-char non-special names pass through as xkb keysym names
        assert_eq!(keysym_name("Prior").unwrap(), "prior"); // lowercased
    }
    #[test]
    fn keysym_f_alone_types_letter() {
        assert_eq!(keysym_name("f").unwrap(), "f");
    }

    // ---- modifier_name ----

    #[test]
    fn modifier_ctrl() {
        assert_eq!(modifier_name("ctrl").unwrap(), "ctrl");
        assert_eq!(modifier_name("Control").unwrap(), "ctrl");
    }
    #[test]
    fn modifier_super() {
        assert_eq!(modifier_name("super").unwrap(), "logo");
        assert_eq!(modifier_name("meta").unwrap(), "logo");
    }
    #[test]
    fn modifier_unknown() {
        assert!(modifier_name("hyper").is_err());
    }

    // ---- toplevels_from_tree ----

    #[test]
    fn tree_parses_views() {
        let tree = serde_json::json!({
            "id": 1, "type": "root", "name": "root",
            "nodes": [{
                "id": 2, "type": "output", "name": "HEADLESS-1",
                "nodes": [{
                    "id": 3, "type": "workspace", "name": "1",
                    "nodes": [{
                        "id": 5, "type": "con", "name": "XTerm",
                        "app_id": null, "pid": 1234,
                        "window_properties": {"class": "XTerm"},
                        "focused": true, "geometry": {"x":0,"y":0,"width":640,"height":480}
                    }]
                }]
            }]
        });
        let views = toplevels_from_tree(&tree);
        assert_eq!(views.len(), 1);
        let (id, name, pid) = &views[0];
        assert_eq!(*id, 5);
        assert_eq!(name, "XTerm");
        assert_eq!(*pid, Some(1234));
    }

    #[test]
    fn tree_empty_workspace() {
        let tree = serde_json::json!({
            "id": 1, "type": "root", "name": "root",
            "nodes": []
        });
        let views = toplevels_from_tree(&tree);
        assert!(views.is_empty());
    }

    // ---- capability matrix: Wayland driver refuses host desktop access ----

    #[test]
    fn a11y_driver_refuses() {
        let driver = WaylandDriver::new(
            "wayland-test".into(),
            PathBuf::from("/tmp/nonexistent"),
            "HEADLESS-1".into(),
            800, 600,
        );
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(driver.get_desktop_overview());
        assert!(result.is_err());
    }

    #[test]
    fn driver_click_element_refuses() {
        let driver = WaylandDriver::new(
            "wayland-test".into(),
            PathBuf::from("/tmp/nonexistent"),
            "HEADLESS-1".into(),
            800, 600,
        );
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(driver.click_element(0, 0, MouseButton::Left));
        assert!(result.is_err());
    }
}

// ---- sway tree parsing ----

pub(crate) fn toplevels_from_tree(tree: &serde_json::Value) -> Vec<(u32, String, Option<u32>)> {
    fn collect(node: &serde_json::Value, out: &mut Vec<(u32, String, Option<u32>)>) {
        let is_view = node.get("app_id").map(|v| v.is_string()).unwrap_or(false)
            || node.get("window_properties").map(|v| v.is_object()).unwrap_or(false);
        if is_view {
            out.push((
                node.get("id").and_then(|v| v.as_i64()).unwrap_or(0) as u32,
                node.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                node.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32),
            ));
        }
        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = node.get(key).and_then(|v| v.as_array()) {
                for c in children { collect(c, out); }
            }
        }
    }
    let mut out = Vec::new();
    collect(tree, &mut out);
    out
}
