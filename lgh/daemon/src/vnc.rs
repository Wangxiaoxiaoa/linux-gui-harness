//! Per-display x11vnc server lifecycle. One x11vnc child serves one
//! sandbox display over the RFB protocol (VNC), giving the user an
//! interactive remote view of the sandbox in any VNC client.

use std::collections::HashMap;

use tokio::sync::Mutex;

use lgh_runtime::ManagedProcess;

/// RFB port range: 5900 + K, K >= 1 (5900 itself is often taken by a
/// system VNC server).
const RFB_PORT_BASE: u16 = 5900;
const RFB_PORT_MAX: u16 = 5999;

/// Track the VNC server child and port per display string.
pub struct VncRegistry {
    servers: Mutex<HashMap<String, VncServer>>,
}

struct VncServer {
    child: ManagedProcess,
    port: u16,
}

impl Default for VncRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VncRegistry {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn a VNC server for `display_str` if one is not already running.
    /// Sway-backed worlds get wayvnc (wlroots screencopy — rootless
    /// XWayland roots reject the X11 grabs x11vnc relies on); Xvfb worlds
    /// keep x11vnc. Returns the RFB port the server listens on.
    pub async fn start(
        &self,
        display_str: &str,
        wayland_display: Option<&str>,
    ) -> Result<u16, lgh_core::LghError> {
        let mut servers = self.servers.lock().await;
        if let Some(existing) = servers.get(display_str) {
            return Ok(existing.port);
        }
        let port = self.pick_port(&servers)?;
        let child = match wayland_display {
            Some(wd) => {
                eprintln!("[vnc] spawning wayvnc {wd} -rfbport {port}");
                // -S: per-instance control socket — wayvnc refuses to start
                // twice otherwise (shared default ctl socket path).
                ManagedProcess::spawn(
                    "wayvnc",
                    &[
                        "-S",
                        &format!("/tmp/lgh-wayvnc-{port}.sock"),
                        "0.0.0.0",
                        &port.to_string(),
                    ],
                    &[("WAYLAND_DISPLAY", wd)],
                )
                .await?
            }
            None => {
                eprintln!("[vnc] spawning x11vnc -display {display_str} -rfbport {port}");
                ManagedProcess::spawn(
                    "x11vnc",
                    &[
                        "-display",
                        display_str,
                        "-rfbport",
                        &port.to_string(),
                        "-shared",  // multiple clients allowed
                        "-forever", // keep serving after client disconnect
                        "-nopw",    // local-only MVP: the sandbox is disposable
                    ],
                    &[],
                )
                .await?
            }
        };
        servers.insert(display_str.to_string(), VncServer { child, port });
        Ok(port)
    }

    /// Stop the x11vnc server of a destroyed display.
    pub async fn stop(&self, display_str: &str) {
        if let Some(mut server) = self.servers.lock().await.remove(display_str) {
            let _ = server.child.kill().await;
        }
    }

    /// Stop every server (daemon shutdown).
    pub async fn stop_all(&self) {
        for (_, mut server) in self.servers.lock().await.drain() {
            let _ = server.child.kill().await;
        }
    }

    /// Find a free RFB port. Ports taken by our own running servers and by
    /// anything listening on the system are both skipped.
    fn pick_port(&self, servers: &HashMap<String, VncServer>) -> Result<u16, lgh_core::LghError> {
        let used: Vec<u16> = servers.values().map(|s| s.port).collect();
        for k in 1..=(RFB_PORT_MAX - RFB_PORT_BASE) {
            let port = RFB_PORT_BASE + k;
            if used.contains(&port) {
                continue;
            }
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Ok(port);
            }
        }
        Err(lgh_core::LghError::InvalidArgument(
            "no free RFB port in 5901-5999".into(),
        ))
    }
}
