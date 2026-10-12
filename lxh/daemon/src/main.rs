use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lxh_daemon::server::DaemonServer;
use lxh_daemon::SessionInfo;
use lxh_runtime::Runtime;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn resolve_socket_path(args: &mut Vec<String>) -> PathBuf {
    let mut idx = None;
    for (i, arg) in args.iter().enumerate() {
        if arg == "--socket" {
            idx = Some(i);
            break;
        }
    }

    if let Some(i) = idx {
        if i + 1 >= args.len() {
            eprintln!("error: --socket requires a value");
            std::process::exit(1);
        }
        let path = args.remove(i + 1);
        args.remove(i);
        return PathBuf::from(path);
    }

    env::var("LXH_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::var("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
                .join("linux-x11-harness.sock")
        })
}

fn pid_path(socket_path: &Path) -> PathBuf {
    let mut p = socket_path.to_path_buf();
    p.set_extension("pid");
    p
}

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = env::args().collect();
    let socket_path = resolve_socket_path(&mut args);
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("mcp");

    if command == "--help" || command == "-h" {
        println!("Usage: linux-x11-harness [serve|mcp|stop|status] [--socket PATH]");
        return;
    }

    match command {
        "serve" => run_serve(socket_path).await,
        "mcp" => run_mcp(socket_path).await,
        "stop" => run_stop(socket_path).await,
        "status" => run_status(socket_path).await,
        _ => {
            eprintln!("Usage: linux-x11-harness [serve|mcp|stop|status] [--socket PATH]");
            std::process::exit(1);
        }
    }
}

async fn run_serve(socket_path: PathBuf) {
    // A SIGKILLed daemon cannot clean up after its displays; leftover
    // sockets and lock files would block those display numbers from ever
    // being reused. Sweep the stale ones before binding our own socket.
    clean_stale_x_displays();

    let pid = std::process::id();
    let _ = tokio::fs::write(pid_path(&socket_path), pid.to_string()).await;

    // Chromium/Electron only render AT-SPI trees when the session advertises
    // accessibility; do it once so apps launched on harness displays are
    // automatable from birth.
    if let Err(e) = lxh_state::a11y::advertise_once() {
        eprintln!("a11y advertisement unavailable: {e}");
    }

    let runtime = Arc::new(Runtime::new());
    let server = DaemonServer::new(runtime, socket_path.clone());
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot install SIGTERM handler: {e}");
            std::process::exit(1);
        }
    };

    tokio::select! {
        result = server.run() => {
            if let Err(e) = result {
                eprintln!("daemon error: {e}");
                std::process::exit(1);
            }
        }
        _ = sigterm.recv() => {
            // Tear down every display (Xvfb, WM, apps, preview) before
            // exiting; `kill_on_drop` never fires on signal death, so a
            // plain exit would leak every Xvfb process.
            eprintln!("SIGTERM received, stopping displays");
            server.shutdown().await;
            let _ = tokio::fs::remove_file(&socket_path).await;
            let _ = tokio::fs::remove_file(pid_path(&socket_path)).await;
        }
    }
}

async fn run_stop(socket_path: PathBuf) {
    let pid_path = pid_path(&socket_path);
    let pid: u32 = match tokio::fs::read_to_string(&pid_path).await {
        Ok(s) => s.trim().parse().unwrap_or(0),
        Err(_) => {
            eprintln!("daemon not running");
            std::process::exit(1);
        }
    };
    if pid == 0 {
        eprintln!("daemon not running");
        std::process::exit(1);
    }
    // Graceful stop via SIGTERM; /bin/kill avoids unsafe libc calls.
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let _ = tokio::fs::remove_file(&pid_path).await;
    let _ = tokio::fs::remove_file(&socket_path).await;
    println!("stopped daemon {pid}");
}

async fn run_status(socket_path: PathBuf) {
    if !socket_path.exists() {
        println!("stopped");
        std::process::exit(1);
    }
    println!("running");
}

/// Remove X11 sockets and lock files whose owning X server is gone.
///
/// A daemon killed with SIGKILL cannot run its shutdown path, leaving
/// `/tmp/.X11-unix/X<N>` and `/tmp/.X<N>-lock` behind; a stale socket
/// blocks the same display number from being reused. The lock file
/// records the X server's PID, so a display counts as stale exactly
/// when that PID is no longer alive. Live servers are never touched.
fn clean_stale_x_displays() {
    let dir = match std::fs::read_dir("/tmp/.X11-unix") {
        Ok(dir) => dir,
        Err(_) => return,
    };
    for entry in dir.flatten() {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        let Some(num) = name.strip_prefix('X') else {
            continue;
        };
        if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let lock_path = std::path::Path::new("/tmp").join(format!(".X{num}-lock"));
        // Without a lock file there is no owner to verify against; leave
        // the socket alone rather than risk deleting a live display's.
        let Ok(lock) = std::fs::read_to_string(&lock_path) else {
            continue;
        };
        let Some(pid) = lock
            .lines()
            .nth(1)
            .and_then(|l| l.trim().parse::<u32>().ok())
        else {
            continue;
        };
        if std::path::Path::new("/proc").join(pid.to_string()).exists() {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
        let _ = std::fs::remove_file(&lock_path);
        eprintln!("[startup] removed stale X display :{num} (pid {pid} gone)");
    }
}

/// Start the daemon in its own session so it is reparented to init
/// immediately. Agents that kill their MCP server's whole process tree on
/// exit (e.g. OpenCode) would otherwise take the shared daemon — and every
/// display inside it — down with them.
async fn spawn_daemon_detached(socket_path: &Path) {
    let executable = env::current_exe().expect("current exe");

    let detached = tokio::process::Command::new("setsid")
        .arg("-f")
        .arg(&executable)
        .arg("serve")
        .arg("--socket")
        .arg(socket_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    if detached.is_err() || !detached.unwrap().success() {
        // setsid unavailable or failed; fall back to a direct child.
        let mut cmd = tokio::process::Command::new(&executable);
        cmd.arg("serve").arg("--socket").arg(socket_path);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.spawn().expect("spawn daemon");
    }

    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if socket_path.exists() {
            return;
        }
    }

    eprintln!("daemon failed to start");
    std::process::exit(1);
}

async fn run_mcp(socket_path: PathBuf) {
    // Connect to the running daemon; if that fails (no daemon yet, or a stale
    // socket file left behind by a killed daemon), start a fresh one. Clients
    // may sanitize the environment, so the resolved path can differ between
    // agents — never assume an existing socket is the right one.
    let mut stream = None;
    for _ in 0..4 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket_path).await {
            stream = Some(s);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let stream = match stream {
        Some(stream) => stream,
        None => {
            let _ = tokio::fs::remove_file(&socket_path).await;
            spawn_daemon_detached(&socket_path).await;
            tokio::net::UnixStream::connect(&socket_path)
                .await
                .expect("connect daemon")
        }
    };

    let (read_half, mut write_half) = stream.into_split();

    // Forward both directions concurrently. Awaiting the stdin->socket
    // direction before polling socket->stdout would deadlock interactive
    // clients until stdin closes.
    let to_stdout = tokio::spawn(forward_plain(read_half, tokio::io::stdout()));
    forward_requests(tokio::io::stdin(), &mut write_half).await;
    let _ = write_half.shutdown().await;
    let _ = to_stdout.await;
}

/// Forward the host's JSON-RPC stream to the daemon, attaching session
/// identity to every request.
///
/// The proxy is the session's representative on the daemon side: it
/// extracts the host's `_meta.threadId`/`sessionId` (codex sends these on
/// every call) and stamps each request with a session envelope the daemon
/// tracks displays by. Responses flow back through `forward_plain`,
/// untouched.
async fn forward_requests<R, W>(reader: R, mut writer: W)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let proxy_pid = std::process::id();
    let mut session = SessionInfo {
        client: None,
        thread_id: None,
        proxy_pid,
    };
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            // Not JSON (or malformed): pass through so the daemon's parser
            // produces the proper error response for the host.
            let _ = writer.write_all(line.as_bytes()).await;
            let _ = writer.write_all(b"\n").await;
            continue;
        };

        match msg.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                if let Some(info) = msg
                    .pointer("/params/clientInfo/name")
                    .and_then(Value::as_str)
                {
                    session.client = Some(info.to_string());
                }
            }
            Some("tools/call") | Some("resources/read") | Some("prompts/get") => {
                // Hosts attach session identity either at params._meta
                // threadId or params._meta.sessionId.
                for key in ["threadId", "sessionId"] {
                    if let Some(id) = msg
                        .pointer("/params/_meta")
                        .and_then(|m| m.get(key))
                        .and_then(Value::as_str)
                    {
                        session.thread_id = Some(id.to_string());
                        break;
                    }
                }
            }
            _ => {}
        }

        let out = json!({
            "session": session,
            "inner": msg,
        });
        if writer
            .write_all(serde_json::to_string(&out).unwrap_or_default().as_bytes())
            .await
            .is_err()
        {
            break;
        }
        if writer.write_all(b"\n").await.is_err() {
            break;
        }
    }
}

/// Daemon-to-host direction: responses and notifications, byte-for-byte.
async fn forward_plain<R, W>(reader: R, mut writer: W)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        if writer.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_stale_x_display_leftovers() {
        // Use a high display number no real server would occupy.
        let num = 59530u32;
        let socket = std::path::Path::new("/tmp/.X11-unix").join(format!("X{num}"));
        let lock = std::path::Path::new("/tmp").join(format!(".X{num}-lock"));
        std::fs::write(&socket, b"").unwrap();
        // Second line of the lock file holds the X server's PID; /proc/0
        // never exists, so this PID reads as dead.
        std::fs::write(&lock, format!("     {num}\n      0\n")).unwrap();

        clean_stale_x_displays();

        assert!(!socket.exists(), "stale socket should be removed");
        assert!(!lock.exists(), "stale lock file should be removed");
    }

    #[test]
    fn keeps_live_x_display_leftovers() {
        let num = 59531u32;
        let socket = std::path::Path::new("/tmp/.X11-unix").join(format!("X{num}"));
        let lock = std::path::Path::new("/tmp").join(format!(".X{num}-lock"));
        std::fs::write(&socket, b"").unwrap();
        // Our own PID is alive, so this display must be kept.
        let own_pid = std::process::id();
        std::fs::write(&lock, format!("     {num}\n      {own_pid}\n")).unwrap();

        clean_stale_x_displays();

        assert!(socket.exists(), "live display socket must be kept");
        assert!(lock.exists(), "live display lock must be kept");

        std::fs::remove_file(&socket).unwrap();
        std::fs::remove_file(&lock).unwrap();
    }
}
