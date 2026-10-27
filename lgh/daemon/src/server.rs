use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lgh_core::LghError;
use lgh_preview::PreviewPanel;
use lgh_runtime::Runtime;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use crate::handlers::{self, session_key, ClientSession, DaemonState};
use crate::protocol::{Request, Response};

pub struct DaemonServer {
    state: Arc<DaemonState>,
    socket_path: PathBuf,
}

impl DaemonServer {
    pub fn new(runtime: Arc<Runtime>, socket_path: PathBuf) -> Self {
        Self {
            state: Arc::new(DaemonState {
                runtime,
                displays: Arc::new(RwLock::new(HashMap::new())),
                drivers: Arc::new(RwLock::new(HashMap::new())),
                vnc: Arc::new(crate::vnc::VncRegistry::new()),
                previews: Arc::new(PreviewPanel::new()),
                zooms: Arc::new(std::sync::Mutex::new(HashMap::new())),
            }),
            socket_path,
        }
    }

    pub async fn run(&self) -> Result<(), LghError> {
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| LghError::InvalidArgument(format!("cannot bind socket: {e}")))?;

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| LghError::InvalidArgument(format!("accept failed: {e}")))?;
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                let mut session = ClientSession::new();
                session.client_name = peer_name(&stream);
                if let Err(e) = handle_client(Arc::clone(&state), &mut session, stream).await {
                    eprintln!("client handler error: {e}");
                }
                // The session is over: harness displays created through this
                // connection are destroyed. Attached external displays survive.
                handlers::destroy_session_displays(&state, &session).await;
            });
        }
    }

    /// Stop every display (and its preview) — the SIGTERM path.
    pub async fn shutdown(&self) {
        let mut displays = self.state.displays.write().await;
        for (id, display) in displays.drain() {
            self.state.previews.close(&id);
            let _ = display.lock().await.destroy().await;
        }
        self.state.drivers.write().await.clear();
        self.state.zooms.lock().unwrap().clear();
        self.state.runtime.clipboard().stop_all();
        self.state.vnc.stop_all().await;
    }
}

async fn handle_client(
    state: Arc<DaemonState>,
    session: &mut ClientSession,
    stream: UnixStream,
) -> Result<(), LghError> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| LghError::InvalidArgument(format!("read failed: {e}")))?;
        if n == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }

        let req = match serde_json::from_str::<Request>(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = Response::error(None, -32700, &format!("invalid json: {e}"));
                let msg = serde_json::to_string(&resp)
                    .map_err(|e| LghError::InvalidArgument(format!("serialize failed: {e}")))?;
                write_half.write_all(msg.as_bytes()).await.ok();
                write_half.write_all(b"\n").await.ok();
                continue;
            }
        };

        let is_notification = req.id.is_none();
        // Sticky identity: an un-attributed request (e.g. lgh_list_displays
        // from a hook-injected shell) must not strip the connection's
        // session key — only an explicit new identity may replace it.
        if let Some(key) = session_key(&req.params) {
            session.session_key = Some(key);
        }
        let resp = dispatch(&state, session, req).await;
        if is_notification {
            continue;
        }
        let msg = serde_json::to_string(&resp)
            .map_err(|e| LghError::InvalidArgument(format!("serialize failed: {e}")))?;
        write_half
            .write_all(msg.as_bytes())
            .await
            .map_err(|e| LghError::InvalidArgument(format!("write failed: {e}")))?;
        write_half
            .write_all(b"\n")
            .await
            .map_err(|e| LghError::InvalidArgument(format!("write failed: {e}")))?;
    }

    Ok(())
}

async fn dispatch(state: &DaemonState, session: &mut ClientSession, req: Request) -> Response {
    let result = match req.method.as_str() {
        "initialize" => {
            let supported = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
            let requested = req.params.get("protocolVersion").and_then(|v| v.as_str());
            let protocol_version = requested
                .and_then(|r| supported.iter().find(|&&s| s == r).copied())
                .unwrap_or("2025-11-25");
            Ok(json!({
                "protocolVersion": protocol_version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "linux-gui-harness", "version": "0.1.0" }
            }))
        }
        "tools/list" => Ok(json!({ "tools": handlers::tool_definitions() })),
        "tools/call" => dispatch_tool_call(state, session, &req.params)
            .await
            .map(tool_result),
        _ => Err(LghError::InvalidArgument(format!(
            "unknown method: {}",
            req.method
        ))),
    };

    match result {
        Ok(value) => Response::result(req.id, value),
        Err(e) => Response::error(req.id, -32603, &e.to_string()),
    }
}

fn tool_result(value: Value) -> Value {
    json!({
        "content": [ { "type": "text", "text": value.to_string() } ]
    })
}

async fn dispatch_tool_call(
    state: &DaemonState,
    session: &mut ClientSession,
    params: &Value,
) -> Result<Value, LghError> {
    let name = params["name"]
        .as_str()
        .ok_or_else(|| LghError::InvalidArgument("tool name required".into()))?;
    let args = &params["arguments"];

    // Cross-session guard: a display with an owner may only be touched by
    // its owning session (identified by the proxy's session envelope).
    // Attached external displays have no owner and stay open to the
    // attaching session's disconnect semantics (never auto-destroyed).
    if let Some(display_id) = args.get("display_id").and_then(Value::as_str) {
        handlers::ensure_display_owner(state, session, display_id).await?;
    }

    match name {
        "lgh_display_create" => handlers::create_display(state, session, args).await,
        "lgh_display_destroy" => handlers::destroy_display(state, args).await,
        "lgh_list_displays" => handlers::list_displays(state).await,
        "lgh_display_attach" => handlers::attach_display(state, args).await,
        "lgh_display_detach" => handlers::detach_display(state, args).await,
        "lgh_app_launch" => handlers::app_launch(state, args).await,
        "lgh_app_terminate" => handlers::app_terminate(state, args).await,
        "lgh_input_click" => handlers::click(state, args).await,
        "lgh_input_move" => handlers::move_mouse(state, args).await,
        "lgh_input_type" => handlers::input_type(state, args).await,
        "lgh_input_key" => handlers::input_key(state, args).await,
        "lgh_input_scroll" => handlers::scroll(state, args).await,
        "lgh_input_drag" => handlers::drag(state, args).await,
        "lgh_input_get_cursor_position" => handlers::get_cursor_position(state, args).await,
        "lgh_zoom" => handlers::zoom(state, args).await,
        "lgh_hover" => handlers::hover(state, args).await,
        "lgh_capture_window" => handlers::screenshot_window(state, args).await,
        "lgh_ocr" => handlers::ocr(state, args).await,
        "lgh_window_focus" => handlers::window_focus(state, args).await,
        "lgh_window_set_frame" => handlers::window_set_frame(state, args).await,
        "lgh_window_close" => handlers::window_close(state, args).await,
        "lgh_click_element" => handlers::click_element(state, args).await,
        "lgh_wait" => handlers::wait(state, args).await,
        "lgh_clipboard_get" => handlers::clipboard_get(state, args).await,
        "lgh_clipboard_set" => handlers::clipboard_set(state, args).await,
        "lgh_display_info" => handlers::display_info(state, args).await,
        "lgh_preview_open" => handlers::preview_open(state, session, args).await,
        "lgh_preview_close" => handlers::preview_close(state, args).await,
        "lgh_session_end" => handlers::session_end(state, session).await,
        "lgh_display_reown" => handlers::reown_display(state, session, args).await,
        "lgh_invoke_menu" => handlers::invoke_menu(state, args).await,
        "lgh_verify_state" => handlers::verify_state(state, args).await,
        "lgh_get_window_state" => handlers::get_window_state(state, args).await,
        "lgh_list_apps" => Ok(handlers::list_apps()),
        "lgh_get_desktop_overview" => handlers::get_desktop_overview(state, args).await,
        "lgh_list_user_windows" => handlers::list_user_windows(state, args).await,
        "lgh_set_value" => handlers::set_value(state, args).await,
        _ => Err(LghError::InvalidArgument(format!("unknown tool: {name}"))),
    }
}

fn peer_name(stream: &UnixStream) -> Option<String> {
    let cred = stream.peer_cred().ok()?;
    let pid = cred.pid()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = comm.trim();
    let friendly = match name {
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "pi" => "Pi",
        _ => return None,
    };
    Some(friendly.to_string())
}
