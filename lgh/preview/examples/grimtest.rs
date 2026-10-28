//! Reproduce the cell's grim spawn outside the panel: pinned stdio layout,
//! env override, blocking `output()`.
use std::process::{Command, Stdio};

fn main() {
    let wd = std::env::args().nth(1).unwrap_or_else(|| "wayland-1".into());
    eprintln!("target wd = {wd:?}");

    // Variant 1: exact cell spawn (stdout piped, stderr null, env override).
    let out = Command::new("grim")
        .arg("-t")
        .arg("ppm")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("WAYLAND_DISPLAY", &wd)
        .output()
        .expect("spawn grim");
    println!(
        "v1 (cell-style): status={:?} out={}B err={:?}",
        out.status.code(),
        out.stdout.len(),
        String::from_utf8_lossy(&out.stderr)
    );

    // Variant 2: default output() stdio (all piped), env override.
    let out = Command::new("grim")
        .arg("-t")
        .arg("ppm")
        .env("WAYLAND_DISPLAY", &wd)
        .output()
        .expect("spawn grim");
    println!(
        "v2 (all piped): status={:?} out={}B err={:?}",
        out.status.code(),
        out.stdout.len(),
        String::from_utf8_lossy(&out.stderr)
    );

    // Variant 3: set env in parent process first, then plain output().
    std::env::set_var("WAYLAND_DISPLAY", &wd);
    let out = Command::new("grim")
        .arg("-t")
        .arg("ppm")
        .output()
        .expect("spawn grim");
    println!(
        "v3 (parent env): status={:?} out={}B err={:?}",
        out.status.code(),
        out.stdout.len(),
        String::from_utf8_lossy(&out.stderr)
    );

    // Variant 4: write to a file instead of stdout — does capture work at all?
    let out = Command::new("grim")
        .arg("-t")
        .arg("ppm")
        .arg("/tmp/rust_grim.ppm")
        .output()
        .expect("spawn grim");
    println!(
        "v4 (file): status={:?} err={:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}
