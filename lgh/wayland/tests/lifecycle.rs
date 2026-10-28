//! Wayland sandbox integration tests.
//!
//! These tests require `sway` in PATH (from the wsp-runtime or system).
//! They exercise the full lifecycle: spawn sway headless → verify socket →
//! run a quick command → teardown → verify cleanup.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Check if sway is available in PATH.
fn sway_available() -> bool {
    std::process::Command::new("sway")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

struct TestWorld {
    pid: u32,
    ipc: PathBuf,
    wayland_display: String,
}

/// Spawn a headless sway and discover its sockets (probe-validated logic).
async fn spawn_headless(world_id: &str) -> Result<TestWorld, String> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let lgh_dir = runtime_dir.join("lgh-test");
    let _ = std::fs::create_dir_all(&lgh_dir);
    let before: Vec<String> = std::fs::read_dir(&runtime_dir)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(String::from))
                .filter(|n| n.starts_with("wayland-") && !n.contains(".lock"))
                .collect()
        })
        .unwrap_or_default();

    let conf = lgh_dir.join(format!("{world_id}.conf"));
    std::fs::write(&conf, "bar {\n mode invisible\n}\n").map_err(|e| e.to_string())?;

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lgh_dir.join(format!("{world_id}.log")))
        .map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = tokio::process::Command::new("sway");
    cmd.arg("-c").arg(&conf);
    cmd.env("WLR_BACKENDS", "headless");
    cmd.env("WLR_LIBINPUT_NO_DEVICES", "1");
    cmd.env("WLR_RENDERER", "pixman");
    cmd.env_remove("WAYLAND_DISPLAY");
    cmd.env_remove("DISPLAY");
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(log);
    cmd.stderr(log_err);

    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn sway: {e}"))?;
    let pid = child.id().unwrap_or(0);
    drop(child);

    // Wait for IPC + new wayland socket.
    let deadline = Instant::now() + Duration::from_secs(8);
    let ipc_path = runtime_dir.join(format!("sway-ipc.1000.{pid}.sock"));
    let mut wayland_display: Option<String> = None;
    loop {
        if std::os::unix::net::UnixStream::connect(&ipc_path).is_ok() {
            let after: Vec<String> = std::fs::read_dir(&runtime_dir)
                .ok()
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().to_str().map(String::from))
                        .filter(|n| n.starts_with("wayland-") && !n.contains(".lock"))
                        .collect()
                })
                .unwrap_or_default();
            let new: Vec<_> = after.iter().filter(|s| !before.contains(s)).cloned().collect();
            if let Some(first) = new.first() {
                wayland_display = Some(first.clone());
            }
        }
        if wayland_display.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
            return Err("sway did not produce a display socket in time".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(TestWorld {
        pid,
        ipc: ipc_path,
        wayland_display: wayland_display.unwrap(),
    })
}

/// SIGTERM-first teardown with socket sweep.
fn teardown(world: &TestWorld) {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &world.pid.to_string()])
        .status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !std::path::Path::new(&format!("/proc/{}", world.pid)).exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if std::path::Path::new(&format!("/proc/{}", world.pid)).exists() {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &world.pid.to_string()])
            .status();
    }
    let _ = std::fs::remove_file(runtime_dir().join(&world.wayland_display));
    let _ = std::fs::remove_file(runtime_dir().join(format!(
        ".{}.lock",
        world.wayland_display
    )));
    let _ = std::fs::remove_file(&world.ipc);
}

#[tokio::test]
async fn sway_headless_lifecycle() {
    if !sway_available() {
        eprintln!("skipping: sway not in PATH");
        return;
    }
    let world = spawn_headless("test-lifecycle").await.expect("spawn sway");

    // Verify the wayland socket file exists.
    let wl_path = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join(&world.wayland_display))
        .unwrap();
    assert!(
        wl_path.exists(),
        "wayland socket {} should exist",
        wl_path.display()
    );

    // Verify the IPC socket accepts connections.
    let ipc = std::os::unix::net::UnixStream::connect(&world.ipc);
    assert!(ipc.is_ok(), "sway IPC socket should accept connections");

    teardown(&world);

    // After teardown, the process should be gone.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !std::path::Path::new(&format!("/proc/{}", world.pid)).exists(),
        "sway process should be gone after teardown"
    );
}

#[tokio::test]
async fn sway_headless_capture_via_grim() {
    if !sway_available() {
        eprintln!("skipping: sway not in PATH");
        return;
    }
    let world = spawn_headless("test-capture").await.expect("spawn sway");

    // Wait for compositor to be fully ready.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Capture via grim — verify we get a non-empty PNG.
    let mut cmd = tokio::process::Command::new("grim");
    cmd.env("WAYLAND_DISPLAY", &world.wayland_display).arg("-");
    let out = cmd.output().await.expect("grim spawn");
    assert!(out.status.success(), "grim should succeed");
    assert!(
        out.stdout.len() > 100,
        "PNG should be non-trivial (got {} bytes)",
        out.stdout.len()
    );
    // PNG magic bytes
    assert_eq!(&out.stdout[..4], b"\x89PNG", "output should be a valid PNG");

    teardown(&world);
}

#[tokio::test]
async fn sway_headless_two_worlds_parallel() {
    if !sway_available() {
        eprintln!("skipping: sway not in PATH");
        return;
    }
    let w1 = spawn_headless("test-par-1").await.expect("spawn sway 1");
    let w2 = spawn_headless("test-par-2").await.expect("spawn sway 2");

    // Two worlds must have different wayland sockets.
    assert_ne!(
        w1.wayland_display, w2.wayland_display,
        "parallel worlds must use different sockets"
    );

    // Both should be alive.
    assert!(
        std::path::Path::new(&format!("/proc/{}", w1.pid)).exists(),
        "world 1 sway should be alive"
    );
    assert!(
        std::path::Path::new(&format!("/proc/{}", w2.pid)).exists(),
        "world 2 sway should be alive"
    );

    teardown(&w1);
    teardown(&w2);
}
