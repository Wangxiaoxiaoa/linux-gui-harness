//! sway compositor backend for Wayland sandbox worlds.
//!
//! Process orchestration only — spawns the sway binary, discovers its
//! sockets, manages teardown. Protocol interaction (virtual pointer,
//! capture, window ops) lives in the `lgh-wayland` crate.
//!
//! Adapted from the probe's `world.rs` (all steps validated).

use std::collections::HashSet;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use lgh_core::LghError;

use crate::process::ManagedProcess;

/// How the sway compositor presents its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwayMode {
    /// Invisible — framebuffer in RAM, no host interaction.
    Headless,
    /// Renders into a native window on the host desktop.
    Nested,
}

/// Everything needed to reach and tear down a running sway world.
#[derive(Clone, Debug)]
pub struct SwayHandle {
    pub pid: u32,
    pub ipc: PathBuf,
    pub wayland_display: String,
    pub x_display: Option<String>,
    pub output: String,
    pub width: u32,
    pub height: u32,
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn wayland_sockets() -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(runtime_dir()) {
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n.starts_with("wayland-") && !n.contains(".lock") {
                    set.insert(n.to_string());
                }
            }
        }
    }
    set
}

fn sway_log_file(world_id: &str) -> std::fs::File {
    let dir = runtime_dir().join("lgh");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("sway-{world_id}.log")))
        .expect("sway log file")
}

/// Spawn a sway compositor and discover its sockets.
pub async fn start(
    world_id: &str,
    mode: SwayMode,
    width: u32,
    height: u32,
) -> Result<(ManagedProcess, SwayHandle), LghError> {
    let dir = runtime_dir().join("lgh");
    let _ = std::fs::create_dir_all(&dir);
    let before = wayland_sockets();

    let conf_path = dir.join(format!("w-{world_id}.sway.conf"));
    std::fs::write(
        &conf_path,
        format!("bar {{\n    mode invisible\n}}\n"),
    )
    .map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;

    let log = sway_log_file(world_id);
    let log_err = log.try_clone().map_err(|e| LghError::DisplayUnavailable(e.to_string()))?;

    let mut cmd = tokio::process::Command::new("sway");
    cmd.arg("-c").arg(&conf_path);
    cmd.env("WLR_LIBINPUT_NO_DEVICES", "1");
    // Software rendering: reliable on VMs and headless hosts where the GPU
    // node is absent or inaccessible (Xvfb was software-rendered too).
    cmd.env("WLR_RENDERER", "pixman");
    match mode {
        SwayMode::Headless => {
            cmd.env("WLR_BACKENDS", "headless");
            cmd.env_remove("WAYLAND_DISPLAY");
        }
        SwayMode::Nested => {
            cmd.env("WLR_BACKENDS", "wayland");
            // Keep WAYLAND_DISPLAY: sway connects to the host compositor.
        }
    }
    cmd.env_remove("DISPLAY");
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    cmd.stdin(Stdio::null());
    cmd.stdout(log);
    cmd.stderr(log_err);

    let child = cmd
        .spawn()
        .map_err(|e| LghError::ProcessSpawnFailed(format!("sway: {e}")))?;
    let sway_pid = child.id().unwrap_or(0);
    let mut proc = ManagedProcess::from_child(child);

    // Wait for the IPC socket (sway-ipc.<uid>.<pid>.sock) and a NEW
    // wayland display socket to appear.
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut ipc: Option<PathBuf> = None;
    let mut wayland_display: Option<String> = None;
    loop {
        for cand in ipc_candidates(sway_pid) {
            if UnixStream::connect(&cand).is_ok() {
                ipc = Some(cand);
            }
        }
        if ipc.is_some() {
            let after = wayland_sockets();
            let new: Vec<_> = after.difference(&before).cloned().collect();
            if let Some(first) = new.first() {
                wayland_display = Some(first.clone());
            }
        }
        if ipc.is_some() && wayland_display.is_some() {
            break;
        }
        if Instant::now() > deadline {
            ManagedProcess::kill(&mut proc).await;
            return Err(LghError::DisplayUnavailable(format!(
                "sway (pid {sway_pid}) did not produce a display socket in time"
            )));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ipc = ipc.unwrap();
    let wayland_display = wayland_display.unwrap();

    // Ensure a virtual output exists and set the mode.
    let swaymsg = |args: &[&str]| -> Result<String, LghError> {
        let out = std::process::Command::new("swaymsg")
            .env("SWAYSOCK", &ipc)
            .args(args)
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
    };

    let outputs_json = swaymsg(&["-t", "get_outputs"])?;
    let mut output_name: Option<String> = None;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&outputs_json) {
        if let Some(arr) = v.as_array() {
            if let Some(o) = arr.first() {
                output_name = o.get("name").and_then(|n| n.as_str()).map(String::from);
            }
        }
    }
    if output_name.is_none() {
        swaymsg(&["create_output"])?;
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let out = swaymsg(&["-t", "get_outputs"])?;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
                if let Some(arr) = v.as_array() {
                    if let Some(o) = arr.first() {
                        output_name = o.get("name").and_then(|n| n.as_str()).map(String::from);
                    }
                }
            }
            if output_name.is_some() {
                break;
            }
            if Instant::now() > deadline {
                return Err(LghError::DisplayUnavailable(
                    "created output did not appear".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let output = output_name.ok_or(LghError::DisplayUnavailable("no output on headless sway".into()))?;
    swaymsg(&["output", &output, "mode", &format!("{width}x{height}")])?;

    // Probe the sandbox XWayland display number (for X11-only apps).
    let probe = dir.join(format!("w-{world_id}.env"));
    let _ = std::fs::remove_file(&probe);
    let mut x_display: Option<String> = None;
    for _ in 0..10 {
        swaymsg(&["exec", &format!("echo $DISPLAY > {}", probe.display())])?;
        if let Ok(txt) = std::fs::read_to_string(&probe) {
            let t = txt.trim();
            if t.starts_with(':') {
                x_display = Some(t.to_string());
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let _ = std::fs::remove_file(&probe);

    let handle = SwayHandle {
        pid: sway_pid,
        ipc: ipc.clone(),
        wayland_display: wayland_display.clone(),
        x_display,
        output: output.clone(),
        width,
        height,
    };
    Ok((proc, handle))
}

/// Teardown: SIGTERM-first (sway cleans its sockets), then SIGKILL,
/// then sweep leftover files.
pub async fn stop(handle: &SwayHandle) -> Result<(), LghError> {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &handle.pid.to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !std::path::Path::new(&format!("/proc/{}", handle.pid)).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if std::path::Path::new(&format!("/proc/{}", handle.pid)).exists() {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &handle.pid.to_string()])
            .status();
    }
    // Sweep socket leftovers.
    let _ = std::fs::remove_file(runtime_dir().join(&handle.wayland_display));
    let _ = std::fs::remove_file(runtime_dir().join(format!(
        ".{}.lock",
        handle.wayland_display
    )));
    let _ = std::fs::remove_file(&handle.ipc);
    Ok(())
}

fn ipc_candidates(pid: u32) -> Vec<PathBuf> {
    let suffix = format!(".{pid}.sock");
    let mut v = Vec::new();
    if let Ok(entries) = std::fs::read_dir(runtime_dir()) {
        for e in entries.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("sway-ipc.") && n.ends_with(&suffix) {
                v.push(runtime_dir().join(n.to_string()));
            }
        }
    }
    v
}
