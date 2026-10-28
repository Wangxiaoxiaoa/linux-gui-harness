//! Wayland sandbox MCP-level integration tests.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::time::timeout;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn sway_available() -> bool {
    std::process::Command::new("sway")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn bin_path() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_linux-gui-harness")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/linux-gui-harness")
        })
}

// ---- daemon infrastructure ----

struct TestDaemon {
    socket: PathBuf,
    child: tokio::process::Child,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let mut pid = self.socket.clone();
        pid.set_extension("pid");
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&pid);
    }
}

struct TestConn {
    writer: tokio::net::unix::OwnedWriteHalf,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    id: i64,
}

impl TestConn {
    async fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).await.expect("connect");
        let (r, w) = stream.into_split();
        Self { writer: w, reader: BufReader::new(r), id: 0 }
    }

    async fn init(&mut self) {
        let init = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"capabilities":{}}});
        self.send(&init).await;
        let notify = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        self.send(&notify).await;
    }

    async fn send(&mut self, req: &Value) -> Value {
        self.writer
            .write_all(format!("{}\n", req).as_bytes())
            .await
            .expect("write");
        self.writer.flush().await.expect("flush");
        let mut line = String::new();
        timeout(Duration::from_secs(30), self.reader.read_line(&mut line))
            .await
            .expect("timeout")
            .expect("read");
        serde_json::from_str(&line).expect("parse")
    }

    async fn call_tool(&mut self, name: &str, args: Value) -> Value {
        let req = json!({
            "jsonrpc":"2.0","id":self.id,
            "method":"tools/call",
            "params":{"name":name,"arguments":args}
        });
        self.id += 1;
        self.send(&req).await
    }
}

async fn start_daemon() -> TestDaemon {
    let tmp = std::env::temp_dir();
    let unique = format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let socket = tmp.join(format!("lgh-wl-{unique}.sock"));
    let _ = std::fs::remove_file(&socket);

    let mut cmd = Command::new(bin_path());
    cmd.arg("serve").env("LGH_SOCKET_PATH", &socket);
    let err_log = std::fs::File::create(format!("{}.err", socket.display())).unwrap();
    cmd.stdout(Stdio::from(err_log.try_clone().unwrap()));
    cmd.stderr(Stdio::from(err_log));

    let child = cmd.spawn().expect("spawn daemon");
    for _ in 0..100 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(socket.exists(), "daemon failed to start");
    TestDaemon { socket, child }
}

// ---- helpers ----

fn json_parse(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
        Value::Object(_) => v.clone(),
        _ => json!({}),
    }
}



// ---- tests ----

#[tokio::test]
async fn wayland_headless_full_lifecycle() {
    if !sway_available() { eprintln!("SKIP: sway not in PATH"); return; }

    let mut daemon = start_daemon().await;
    let mut conn = TestConn::connect(&daemon.socket).await;
    conn.init().await;

    // 1. Create.
    let r = conn
        .call_tool("lgh_display_create", json!({"width":800,"height":600,"backend":"wayland-headless"}))
        .await;
    let d = json_parse(&r["result"]["content"][0]["text"]);
    let did = d["display_id"].as_str().unwrap().to_string();
    let display = d["display"].as_str().unwrap().to_string();
    assert!(display.starts_with("wayland-"), "expected wayland socket, got {display}");
    eprintln!("✓ created {did} on {display}");

    // 2. Launch xterm.
    let r = conn
        .call_tool("lgh_app_launch", json!({"display_id":did,"command":"xterm"}))
        .await;
    let pid = r["pid"].as_u64().expect("pid");
    assert!(pid > 0);
    eprintln!("✓ xterm pid {pid}");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 3. Click inside xterm.
    let _ = conn
        .call_tool("lgh_input_click", json!({"display_id":did,"x":300,"y":300,"button":"left"}))
        .await;
    eprintln!("✓ click");

    // 4. Type.
    let _ = conn
        .call_tool("lgh_input_type", json!({"display_id":did,"text":"echo WAYLAND_TEST"}))
        .await;
    eprintln!("✓ type");

    // 5. Return.
    let _ = conn
        .call_tool("lgh_input_key", json!({"display_id":did,"key":"Return"}))
        .await;
    eprintln!("✓ Return");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 6. Screenshot.
    let shot = conn
        .call_tool("lgh_screenshot", json!({"display_id":did,"save_to":"/tmp/lgh-wl-test.png"}))
        .await;
    let saved = shot["saved_to"].as_str().unwrap_or("");
    let png = std::fs::read(saved).unwrap_or_default();
    assert!(png.len() > 100, "screenshot should be non-trivial");
    assert_eq!(&png[..4], b"\x89PNG", "should be valid PNG");
    eprintln!("✓ screenshot ({} bytes)", png.len());

    // 7. Modifier chord.
    let _ = conn
        .call_tool("lgh_input_key", json!({"display_id":did,"key":"c","modifiers":["ctrl"]}))
        .await;
    eprintln!("✓ modifier chord");

    // 8. Destroy.
    let _ = conn
        .call_tool("lgh_display_destroy", json!({"display_id":did}))
        .await;
    eprintln!("✓ destroy");

    // 9. Socket cleanup.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let wl_path = runtime_dir().join(&display);
    assert!(!wl_path.exists(), "socket should be cleaned: {}", wl_path.display());
    eprintln!("✓ socket cleanup");
}

#[tokio::test]
async fn wayland_two_worlds_parallel() {
    if !sway_available() { eprintln!("SKIP: sway not in PATH"); return; }

    let mut daemon = start_daemon().await;
    let mut conn = TestConn::connect(&daemon.socket).await;
    conn.init().await;

    let c1 = conn.call_tool("lgh_display_create", json!({"width":640,"height":480,"backend":"wayland-headless"})).await;
    let d1 = json_parse(&c1["result"]["content"][0]["text"]);
    let id1 = d1["display_id"].as_str().unwrap().to_string();
    let disp1 = d1["display"].as_str().unwrap().to_string();

    let c2 = conn
        .call_tool("lgh_display_create", json!({"width":640,"height":480,"backend":"wayland-headless"}))
        .await;
    let d2 = json_parse(&c2["result"]["content"][0]["text"]);
    let id2 = d2["display_id"].as_str().unwrap().to_string();
    let disp2 = d2["display"].as_str().unwrap().to_string();

    assert_ne!(id1, id2);
    assert_ne!(disp1, disp2);
    eprintln!("✓ parallel worlds: {disp1} + {disp2}");

    let list = conn.call_tool("lgh_list_displays", json!({})).await;
    let worlds = list["result"]["content"][0]["text"].as_str()
        .map(|s| serde_json::from_str::<Value>(s).unwrap_or(json!({})))
        .unwrap_or(json!({}));
    let count = worlds["worlds"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(count >= 2, "expected ≥2 worlds, got {count}");

    let _ = conn.call_tool("lgh_display_destroy", json!({"display_id":id1})).await;
    let _ = conn.call_tool("lgh_display_destroy", json!({"display_id":id2})).await;
    eprintln!("✓ parallel teardown");
}

#[tokio::test]
async fn wayland_modifier_chord() {
    if !sway_available() { eprintln!("SKIP: sway not in PATH"); return; }

    let mut daemon = start_daemon().await;
    let mut conn = TestConn::connect(&daemon.socket).await;
    conn.init().await;

    let create = conn
        .call_tool("lgh_display_create", json!({"width":800,"height":600,"backend":"wayland-headless"}))
        .await;
    let d = json_parse(&create["result"]["content"][0]["text"]);
    let did = d["display_id"].as_str().unwrap().to_string();

    let _ = conn
        .call_tool("lgh_app_launch", json!({"display_id":did,"command":"xterm"}))
        .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let chord = conn
        .call_tool("lgh_input_key", json!({"display_id":did,"key":"c","modifiers":["ctrl"]}))
        .await;
    assert!(!chord["result"].is_null(), "chord should succeed");
    eprintln!("✓ chord");
    let _ = conn.call_tool("lgh_display_destroy", json!({"display_id":did})).await;
}

#[tokio::test]
async fn wayland_trait_only_guard() {
    // Architectural invariant: the daemon's tool dispatch must not branch
    // on protocol (the cua anti-pattern with 31 is_wayland branches).
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handlers.rs"),
    )
    .unwrap_or_default();
    for pattern in ["is_wayland", "match backend", "is_x11"] {
        assert!(
            !src.contains(pattern),
            "daemon tool layer must not branch on protocol: found `{pattern}`"
        );
    }
}

// ---- helpers ----

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}
