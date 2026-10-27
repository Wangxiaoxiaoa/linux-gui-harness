use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin_path() -> PathBuf {
    env::var("CARGO_BIN_EXE_linux-gui-harness")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/linux-gui-harness")
        })
}

#[test]
fn help_flag_returns_zero() {
    for flag in ["--help", "-h"] {
        let output = Command::new(bin_path())
            .arg(flag)
            .output()
            .expect("spawn binary");
        assert!(
            output.status.success(),
            "{flag} should exit 0, got {:?}",
            output.status.code()
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("serve|mcp|stop|status"),
            "help text missing: {stdout}"
        );
    }
}

#[test]
fn unknown_command_returns_non_zero() {
    let output = Command::new(bin_path())
        .arg("not-a-command")
        .output()
        .expect("spawn binary");
    assert!(!output.status.success(), "unknown command should fail");
}

#[test]
fn status_reports_stopped_for_unused_socket() {
    let output = Command::new(bin_path())
        .args(["status", "--socket", "/tmp/lgh-cli-status-test.sock"])
        .output()
        .expect("spawn binary");
    assert!(
        !output.status.success(),
        "status should fail when daemon not running"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("stopped"),
        "expected 'stopped', got: {stdout}"
    );
}
