use std::collections::HashMap;
use std::sync::Arc;

use crate::tools::{
    parse_args, tool_definitions as tools_tool_definitions, AppLaunchArgs, AppTerminateArgs,
    CaptureWindowArgs, ClickArgs, ClickElementArgs, ClipboardSetArgs, DesktopOverviewArgs,
    DisplayCreateArgs, DisplayIdArgs, DragArgs, GetWindowStateArgs, HoverArgs, InvokeMenuArgs,
    KeyArgs, ListUserWindowsArgs, MoveArgs, OcrArgs, ScrollArgs, SetValueArgs, SetWindowFrameArgs,
    TypeArgs, VerifyStateArgs, WaitArgs, WindowIdArgs, ZoomArgs,
};
use lgh_core::{
    Driver, ElementExpectation, LghError, MouseButton, StateExpectation, WindowExpectation,
};
use lgh_preview::PreviewPanel;
use lgh_runtime::{Display, DisplayConfig, Runtime};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tokio::task;

pub struct DaemonState {
    pub runtime: Arc<Runtime>,
    pub displays: Arc<RwLock<HashMap<String, Arc<Mutex<Display>>>>>,
    pub drivers: Arc<RwLock<HashMap<String, Arc<dyn Driver>>>>,
    pub vnc: Arc<crate::vnc::VncRegistry>,
    pub previews: Arc<PreviewPanel>,
    /// Last zoom context per display, for `from_zoom` coordinate
    /// translation in the coordinate-taking input tools.
    pub zooms: Arc<std::sync::Mutex<HashMap<String, ZoomContext>>>,
}

/// Mapping from zoom-image coordinates back to display coordinates: the
/// zoom output's top-left sits at (`display_x`, `display_y`) and one zoom
/// pixel spans `scale` display pixels.
#[derive(Clone, Copy)]
pub struct ZoomContext {
    pub display_x: i32,
    pub display_y: i32,
    pub scale: f64,
}

pub struct ClientSession {
    pub client_name: Option<String>,
    /// Session identity of the agent session: parsed from the request's
    /// `params._meta` (codex) or `arguments._meta` (hook-injected shells)
    /// on every call. Displays created through this session record it as
    /// their owner.
    pub session_key: Option<String>,
    /// Displays created on this connection WITHOUT any session envelope.
    /// They have no owner, so only the connection that created them may
    /// reclaim them — another connection disconnecting must not.
    pub created_unowned: Vec<String>,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientSession {
    pub fn new() -> Self {
        Self {
            client_name: None,
            session_key: None,
            created_unowned: Vec::new(),
        }
    }
}

fn parse_button(b: Option<crate::tools::ButtonArg>) -> Result<MouseButton, LghError> {
    match b.unwrap_or(crate::tools::ButtonArg::Left) {
        crate::tools::ButtonArg::Left => Ok(MouseButton::Left),
        crate::tools::ButtonArg::Right => Ok(MouseButton::Right),
        crate::tools::ButtonArg::Middle => Ok(MouseButton::Middle),
    }
}

pub async fn create_display(
    state: &DaemonState,
    session: &mut ClientSession,
    args: &Value,
) -> Result<Value, LghError> {
    let args: DisplayCreateArgs = parse_args(args)?;
    let mut config = DisplayConfig::default();
    if let Some(b) = &args.backend {
        config.backend = match b.as_str() {
            "wayland-headless" => lgh_runtime::Backend::SwayHeadless,
            "wayland-nested" => lgh_runtime::Backend::SwayNested,
            _ => lgh_runtime::Backend::Xvfb,
        };
    }
    let preview = args.preview.unwrap_or(true);

    let display = state.runtime.create_display(config).await?;
    let id = display.id().to_string();
    let display_str = display.display().to_string();
    let driver = display.driver_handle();

    if let Some(key) = &session.session_key {
        display.set_owner(key.clone());
    } else {
        session.created_unowned.push(id.clone());
    }

    if preview {
        let title = preview_title(session, &args.name);
        if let Err(e) = state.previews.open(&id, &display_str, &title) {
            eprintln!("preview unavailable for {id}: {e}");
        }
    }

    state
        .displays
        .write()
        .await
        .insert(id.clone(), Arc::new(Mutex::new(display)));
    state.drivers.write().await.insert(id.clone(), driver);
    let rfb_port = state.vnc.start(&display_str).await?;
    state.previews.set_vnc_port(&id, rfb_port);

    Ok(json!({ "display_id": id, "display": display_str }))
}

pub async fn attach_display(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let display = state.runtime.attach_display(&args.display_id).await?;
    let id = display.id().to_string();
    let display_str = display.display().to_string();
    let driver = display.driver_handle();

    state
        .displays
        .write()
        .await
        .insert(id.clone(), Arc::new(Mutex::new(display)));
    state.drivers.write().await.insert(id.clone(), driver);
    state.runtime.clipboard().start_spoke(&display_str);

    Ok(json!({ "display_id": id, "display": display_str }))
}

/// Session identity of one tool call, parsed from the request itself:
/// codex attaches `params._meta.sessionId` on every call (its subagent
/// threads share the session id); hook-based shells (pi, opencode) inject
/// `arguments._meta.session_id` into the tool arguments. Thread ids are
/// deliberately ignored — displays bind to session ids only. Calls without
/// a session id are unattributed.
pub fn session_key(params: &Value) -> Option<String> {
    let meta = &params["_meta"];
    if let Some(id) = meta.get("sessionId").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    // Hook shells (pi, opencode) inject the session identity into the
    // tool arguments at the same path codex uses at the params level.
    let arguments = &params["arguments"];
    for key in ["sessionId", "session_id"] {
        if let Some(id) = arguments
            .get("_meta")
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
        {
            return Some(id.to_string());
        }
    }
    None
}

/// Cross-session guard: reject tool calls that touch a display owned by
/// a different session. Displays created before the session-key mechanism
/// (or without any session envelope) have no owner and stay open to all
/// connections; attached external displays are likewise unowned.
pub async fn ensure_display_owner(
    state: &DaemonState,
    session: &ClientSession,
    display_id: &str,
) -> Result<(), LghError> {
    // Not in the registry (e.g. lgh_display_attach reaching for an
    // external display): nothing owned to check against.
    let Ok(display) = find_display(state, display_id).await else {
        return Ok(());
    };
    let owner = display.lock().await.owner();
    let Some(owner) = owner else {
        return Ok(());
    };
    match &session.session_key {
        Some(key) if *key == owner => Ok(()),
        other => Err(LghError::InvalidArgument(format!(
            "display {display_id} belongs to another session{}; \
             create a new display for this session instead",
            other
                .as_ref()
                .map(|k| format!(" (this session: {k})"))
                .unwrap_or_default(),
        ))),
    }
}

/// Destroy the harness displays a client session leaves behind, called
/// when that session's connection closes (or via `lgh_session_end`).
/// Displays owned by *other* sessions are left alone. Attached external
/// displays are always left alone — they were not created here and
/// outlive any single session.
///
/// Unowned displays (created without a session envelope: an older proxy,
/// or a direct socket client) can only be reclaimed by the connection
/// that created them — any other connection disconnecting must leave
/// them alone.
pub async fn destroy_session_displays(state: &DaemonState, session: &ClientSession) {
    let session_key = session.session_key.as_deref();
    let mut ids = Vec::new();
    {
        let displays = state.displays.read().await;
        for (id, display) in displays.iter() {
            let display = display.lock().await;
            if display.is_external() {
                continue;
            }
            match display.owner() {
                // Owned by this session.
                Some(owner) if session_key == Some(owner.as_str()) => ids.push(id.clone()),
                // Unowned display created by this very connection.
                None if session.created_unowned.contains(id) => ids.push(id.clone()),
                // Owned by a different (still-connected) session, or
                // unowned but created elsewhere — not ours to reclaim.
                _ => {}
            }
        }
    }
    for id in ids {
        let display = match find_display(state, &id).await {
            Ok(d) => d,
            Err(_) => continue,
        };
        let display_str = display.lock().await.display().to_string();
        // Unregister first: the display is logically gone the moment the
        // session ends; teardown then runs without holding up observers.
        state.displays.write().await.remove(&id);
        state.drivers.write().await.remove(&id);
        state.zooms.lock().unwrap().remove(&id);
        state.previews.close(&id);
        let _ = display.lock().await.destroy().await;
        state.vnc.stop(&display_str).await;
        state.runtime.clipboard().stop(&display_str);
    }
}

/// Explicit end-of-session request from the model or a host hook: destroy
/// the harness displays this session leaves behind — those it owns, plus
/// any unowned displays it created on this connection. With neither, there
/// is nothing to attribute — report that instead of guessing.
pub async fn session_end(state: &DaemonState, session: &ClientSession) -> Result<Value, LghError> {
    if session.session_key.is_none() && session.created_unowned.is_empty() {
        return Err(LghError::InvalidArgument(
            "no session identity available; nothing to end".into(),
        ));
    }
    destroy_session_displays(state, session).await;
    Ok(json!({ "success": true }))
}

/// Hand a display to another session (e.g. a forked one). Only the
/// current owner may reown: the caller's session identity must match the
/// display's owner, so the cross-session guard stays intact.
pub async fn reown_display(
    state: &DaemonState,
    session: &ClientSession,
    args: &Value,
) -> Result<Value, LghError> {
    let args: crate::tools::ReownArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let owner = display.lock().await.owner();
    match (&session.session_key, &owner) {
        (Some(key), Some(owner_key)) if key == owner_key => {}
        _ => {
            return Err(LghError::InvalidArgument(format!(
                "reown requires the current owner's session identity (display {} is owned by {})",
                args.display_id,
                owner.as_deref().unwrap_or("<unowned>")
            )))
        }
    }
    display.lock().await.set_owner(args.to.clone());
    Ok(json!({ "display_id": args.display_id, "owner": args.to }))
}

pub async fn destroy_display(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let display_str = display.lock().await.display().to_string();
    state.previews.close(&args.display_id);
    display.lock().await.destroy().await?;

    state.displays.write().await.remove(&args.display_id);
    state.drivers.write().await.remove(&args.display_id);
    state.zooms.lock().unwrap().remove(&args.display_id);
    state.vnc.stop(&display_str).await;
    state.runtime.clipboard().stop(&display_str);

    Ok(json!({ "success": true }))
}

pub async fn list_displays(state: &DaemonState) -> Result<Value, LghError> {
    let displays = state.displays.read().await;
    let mut list = Vec::new();
    for (id, display) in displays.iter() {
        let info = display.lock().await.info();
        let owner = display.lock().await.owner();
        list.push(json!({
            "display_id": id,
            "display": info.display,
            "width": info.width,
            "height": info.height,
            "app_count": info.app_count,
            "owner": owner,
        }));
    }
    Ok(json!({ "displays": list, "count": list.len() }))
}

pub async fn preview_open(
    state: &DaemonState,
    session: &ClientSession,
    args: &Value,
) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let display_str = display.lock().await.display().to_string();
    let title = preview_title(session, &None);
    state
        .previews
        .open(&args.display_id, &display_str, &title)?;
    Ok(json!({ "success": true }))
}

pub async fn preview_close(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    state.previews.close(&args.display_id);
    Ok(json!({ "success": true }))
}

fn preview_title(session: &ClientSession, name: &Option<String>) -> String {
    let who = name
        .clone()
        .or_else(|| session.client_name.clone())
        .unwrap_or_else(|| "Agent".to_string());
    format!("LGH {who}")
}

pub async fn detach_display(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let display_str = display.lock().await.display().to_string();
    if !display.lock().await.is_external() {
        return Err(LghError::InvalidArgument(
            "cannot detach a harness display; use lgh_display_destroy".into(),
        ));
    }

    state.displays.write().await.remove(&args.display_id);
    state.drivers.write().await.remove(&args.display_id);
    state.zooms.lock().unwrap().remove(&args.display_id);
    state.runtime.clipboard().stop(&display_str);

    Ok(json!({ "success": true }))
}

pub async fn app_launch(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: AppLaunchArgs = parse_args(args)?;
    let arg_refs: Vec<&str> = args.args.iter().map(|s| s.as_str()).collect();
    let display = find_display(state, &args.display_id).await?;
    let pid = display
        .lock()
        .await
        .launch_app(&args.command, &arg_refs)
        .await?;
    Ok(json!({ "pid": pid }))
}

pub async fn app_terminate(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: AppTerminateArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    display.lock().await.terminate_app(args.pid as u32).await?;
    Ok(json!({ "success": true }))
}

pub async fn click(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ClickArgs = parse_args(args)?;
    let (x, y) = resolve_coord(
        state,
        &args.display_id,
        args.x,
        args.y,
        args.from_zoom.unwrap_or(false),
    )?;
    let driver = find_driver(state, &args.display_id).await?;
    driver
        .click(
            x as i32,
            y as i32,
            parse_button(args.button)?,
            args.count.unwrap_or(1) as u32,
        )
        .await?;
    Ok(json!({ "success": true }))
}

pub async fn move_mouse(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: MoveArgs = parse_args(args)?;
    let (x, y) = resolve_coord(
        state,
        &args.display_id,
        args.x,
        args.y,
        args.from_zoom.unwrap_or(false),
    )?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.move_mouse(x as i32, y as i32).await?;
    Ok(json!({ "success": true }))
}

pub async fn input_type(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: TypeArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.type_text(args.pid, &args.text).await?;
    Ok(json!({ "success": true }))
}

pub async fn input_key(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: KeyArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let modifiers: Vec<&str> = args.modifiers.iter().map(|s| s.as_str()).collect();
    driver.key(&args.key, &modifiers).await?;
    Ok(json!({ "success": true }))
}

pub async fn scroll(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ScrollArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.scroll(args.dx as i32, args.dy as i32).await?;
    Ok(json!({ "success": true }))
}

pub async fn drag(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DragArgs = parse_args(args)?;
    let from_zoom = args.from_zoom.unwrap_or(false);
    let (x1, y1) = resolve_coord(state, &args.display_id, args.x1, args.y1, from_zoom)?;
    let (x2, y2) = resolve_coord(state, &args.display_id, args.x2, args.y2, from_zoom)?;
    let driver = find_driver(state, &args.display_id).await?;
    let modifiers: Vec<String> = args.modifiers;
    let mrefs: Vec<&str> = modifiers.iter().map(|s| s.as_str()).collect();
    driver
        .drag(x1 as i32, y1 as i32, x2 as i32, y2 as i32, &mrefs)
        .await?;
    Ok(json!({ "success": true }))
}

pub async fn get_cursor_position(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let (x, y) = driver.get_cursor_position().await?;
    Ok(json!({ "x": x, "y": y }))
}

pub async fn screenshot_full(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: crate::tools::ScreenshotArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let shot = driver.screenshot().await?;
    if let Some(path) = &args.save_to {
        let bytes = save_png(&shot.data, path)?;
        return Ok(json!({ "saved_to": path, "bytes": bytes }));
    }
    Ok(json!({ "mimeType": "image/png", "data": encode_png(&shot.data) }))
}

pub async fn screenshot_window(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: CaptureWindowArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let shot = driver.screenshot_window(args.window_id as u32).await?;
    if let Some(path) = &args.save_to {
        let bytes = save_png(&shot.data, path)?;
        return Ok(json!({ "saved_to": path, "bytes": bytes }));
    }
    Ok(json!({ "mimeType": "image/png", "data": encode_png(&shot.data) }))
}

/// Persist a PNG and return its size in bytes.
fn save_png(data: &[u8], path: &str) -> Result<usize, LghError> {
    std::fs::write(path, data)
        .map(|_| data.len())
        .map_err(|e| LghError::InvalidArgument(format!("cannot write {}: {e}", path)))
}

pub async fn zoom(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ZoomArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let capture = driver
        .capture_region(args.window_id as u32, args.x1, args.y1, args.x2, args.y2)
        .await?;

    let scale = capture.scale;
    state.zooms.lock().unwrap().insert(
        args.display_id.clone(),
        ZoomContext {
            display_x: capture.display_x,
            display_y: capture.display_y,
            scale,
        },
    );

    if let Some(path) = &args.save_to {
        let bytes = save_png(&capture.screenshot.data, path)?;
        return Ok(json!({
            "saved_to": path,
            "bytes": bytes,
            "zoom": {
                "display_origin": { "x": capture.display_x, "y": capture.display_y },
                "scale": scale,
            }
        }));
    }

    Ok(json!({
        "mimeType": "image/png",
        "data": encode_png(&capture.screenshot.data),
        "zoom": {
            "display_origin": { "x": capture.display_x, "y": capture.display_y },
            "scale": scale,
        }
    }))
}

/// Local OCR engines, probed in order of preference.
enum OcrEngine {
    /// `tesseract <img> stdout -l <lang>`.
    Tesseract,
    /// `python3 -c "from rapidocr_onnxruntime import RapidOCR ..."`.
    RapidOcr,
}

fn detect_ocr_engine() -> Option<OcrEngine> {
    if which_exists("tesseract") {
        return Some(OcrEngine::Tesseract);
    }
    if which_exists("python3")
        && std::process::Command::new("python3")
            .args(["-c", "import rapidocr_onnxruntime"])
            .output()
            .is_ok_and(|o| o.status.success())
    {
        return Some(OcrEngine::RapidOcr);
    }
    None
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .is_ok_and(|o| o.status.success())
}

pub async fn ocr(_state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: OcrArgs = parse_args(args)?;
    if !std::path::Path::new(&args.image_path).exists() {
        return Err(LghError::InvalidArgument(format!(
            "image not found: {}",
            args.image_path
        )));
    }
    let engine = detect_ocr_engine().ok_or_else(|| {
        LghError::InvalidArgument(
            "no OCR engine available; install tesseract (tesseract-ocr \
             tesseract-ocr-chi-sim tesseract-ocr-eng) or the python package \
             rapidocr_onnxruntime"
                .into(),
        )
    })?;

    let text = task::spawn_blocking(move || match engine {
        OcrEngine::Tesseract => {
            let lang = args.lang.clone().unwrap_or_else(|| "chi_sim+eng".into());
            std::process::Command::new("tesseract")
                .args([&args.image_path, "stdout", "-l", &lang])
                .output()
                .map_err(|e| LghError::InvalidArgument(format!("tesseract failed: {e}")))
                .and_then(|o| {
                    if o.status.success() {
                        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
                    } else {
                        Err(LghError::InvalidArgument(format!(
                            "tesseract error: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        )))
                    }
                })
        }
        OcrEngine::RapidOcr => {
            let script = format!(
                "from rapidocr_onnxruntime import RapidOCR;\
                 r=RapidOCR(); res,_=r({:?});\
                 print('\\n'.join(x[1] for x in res) if res else '', end='')",
                args.image_path
            );
            std::process::Command::new("python3")
                .args(["-c", &script])
                .output()
                .map_err(|e| LghError::InvalidArgument(format!("rapidocr failed: {e}")))
                .and_then(|o| {
                    if o.status.success() {
                        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
                    } else {
                        Err(LghError::InvalidArgument(format!(
                            "rapidocr error: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        )))
                    }
                })
        }
    })
    .await
    .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))??;

    Ok(json!({
        "text": text.trim(),
        "chars": text.trim().chars().count(),
    }))
}

/// Translate zoom-image coordinates to display coordinates.
fn resolve_coord(
    state: &DaemonState,
    display_id: &str,
    x: f64,
    y: f64,
    from_zoom: bool,
) -> Result<(i64, i64), LghError> {
    if !from_zoom {
        return Ok((x as i64, y as i64));
    }
    let ctx = state
        .zooms
        .lock()
        .unwrap()
        .get(display_id)
        .copied()
        .ok_or_else(|| {
            LghError::InvalidArgument(
                "from_zoom requires a lgh_zoom call on this display first".into(),
            )
        })?;
    let dx = ctx.display_x + (x / ctx.scale).round() as i32;
    let dy = ctx.display_y + (y / ctx.scale).round() as i32;
    Ok((dx as i64, dy as i64))
}

pub async fn invoke_menu(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: InvokeMenuArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.invoke_menu(args.pid, &args.path).await?;
    Ok(json!({ "success": true }))
}

pub async fn verify_state(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    use lgh_core::{UnknownReason, VerificationStatus};

    let args: VerifyStateArgs = parse_args(args)?;
    for (i, e) in args.expect.iter().enumerate() {
        if let Some(w) = &e.window {
            if w.exists == Some(false) {
                return Err(LghError::InvalidArgument(format!(
                    "expect[{i}].window.exists=false cannot be verified (absence is not \
                     provable); only true is accepted"
                )));
            }
        }
    }
    if args.expect.is_empty() || args.expect.len() > 8 {
        return Err(LghError::InvalidArgument(
            "expect requires 1 to 8 predicates".into(),
        ));
    }

    let driver = find_driver(state, &args.display_id).await?;
    let expect: Vec<StateExpectation> = args
        .expect
        .iter()
        .map(|e| StateExpectation {
            window: e.window.as_ref().map(|w| WindowExpectation {
                exists: w.exists.unwrap_or(true),
                title_contains: w.title_contains.clone(),
            }),
            element: e.element.as_ref().map(|el| ElementExpectation {
                role: el.role.clone(),
                label_contains: el.label_contains.clone(),
            }),
        })
        .collect();

    // Sampling loop: one pass when timeout_ms is unset (v1 behavior);
    // otherwise sample every 200ms until the predicates hold across
    // stable_samples consecutive samples or the deadline passes.
    let timeout_ms = args.timeout_ms.unwrap_or(0);
    let stable_needed = args.stable_samples.unwrap_or(1).max(1);
    let deadline = if timeout_ms > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms))
    } else {
        None
    };
    let interval = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();

    let mut consecutive: u64 = 0;
    let mut samples: u64 = 0;
    let mut last: Option<lgh_core::VerificationResult> = None;
    loop {
        let sample = driver.verify_state(args.pid, &expect).await?;
        samples += 1;
        let all_satisfied = sample
            .predicates
            .iter()
            .all(|p| p.status == VerificationStatus::Satisfied);
        consecutive = if all_satisfied { consecutive + 1 } else { 0 };
        last = Some(sample);
        let proven = all_satisfied && consecutive >= stable_needed;
        match deadline {
            Some(d) => {
                if proven || std::time::Instant::now() >= d {
                    break;
                }
            }
            None => break,
        }
        tokio::time::sleep(interval).await;
    }
    let result = last.expect("at least one sample taken");

    let all_satisfied = result
        .predicates
        .iter()
        .all(|p| p.status == VerificationStatus::Satisfied);
    let stable = all_satisfied && consecutive >= stable_needed;
    let (status, _unknown_reason) = if all_satisfied && stable {
        ("satisfied", None)
    } else if all_satisfied {
        ("unknown", Some(UnknownReason::StabilityUnproven))
    } else if result
        .predicates
        .iter()
        .any(|p| p.status == VerificationStatus::Unknown)
    {
        // Observation failed for at least one predicate: report unknown with
        // that predicate's reason (observation problems are retryable;
        // unsatisfied predicates are not).
        let reason = result
            .predicates
            .iter()
            .find(|p| p.status == VerificationStatus::Unknown)
            .and_then(|p| p.unknown_reason);
        ("unknown", reason)
    } else {
        ("unsatisfied", None)
    };

    let predicates: Vec<Value> = result
        .predicates
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut node = json!({
                "index": i,
                "status": match p.status {
                    VerificationStatus::Satisfied => "satisfied",
                    VerificationStatus::Unsatisfied => "unsatisfied",
                    VerificationStatus::Unknown => "unknown",
                },
                "observed": p.observed,
            });
            if let Some(reason) = p.unknown_reason {
                node["unknown_reason"] = json!(match reason {
                    UnknownReason::InvalidPredicate => "InvalidPredicate",
                    UnknownReason::TargetMissing => "TargetMissing",
                    UnknownReason::ObservationUnavailable => "ObservationUnavailable",
                    UnknownReason::StabilityUnproven => "StabilityUnproven",
                });
            }
            node
        })
        .collect();

    let mut out = json!({
        "status": status,
        "stable": stable,
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "samples": samples,
        "predicates": predicates,
    });
    if args.include_screenshot.unwrap_or(false) {
        if let Some(window_id) = result.matched_window_id {
            if let Ok(shot) = driver.screenshot_window(window_id).await {
                out["screenshot"] = json!(
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.data)
                );
            }
        }
    }
    Ok(out)
}

pub async fn hover(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: HoverArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let duration = std::time::Duration::from_millis(args.duration_ms.unwrap_or(1500));

    // Move mouse to the target position.
    driver.move_mouse(args.x as i32, args.y as i32).await?;

    // Hold: give tooltip time to appear.
    tokio::time::sleep(duration).await;

    // Capture the window under the cursor (or full display if on desktop).
    let (window_id, capture) = driver.capture_at_cursor().await?;
    let capture_type = if window_id > 0 { "window" } else { "full" };

    Ok(json!({
        "mimeType": "image/png",
        "data": encode_png(&capture.data),
        "cursor": { "x": args.x, "y": args.y },
        "capture": capture_type,
    }))
}

pub async fn window_focus(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: WindowIdArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.focus_window(args.window_id as u32).await?;
    Ok(json!({ "success": true }))
}

pub async fn window_set_frame(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: SetWindowFrameArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver
        .set_window_frame(
            args.window_id as u32,
            args.x as i32,
            args.y as i32,
            args.width as u32,
            args.height as u32,
        )
        .await?;
    Ok(json!({ "success": true }))
}

pub async fn window_close(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: WindowIdArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver.close_window(args.window_id as u32).await?;
    Ok(json!({ "success": true }))
}

pub async fn click_element(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ClickElementArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    driver
        .click_element(
            args.pid as u32,
            args.index as usize,
            parse_button(args.button)?,
        )
        .await?;
    Ok(json!({ "success": true }))
}

pub async fn wait(_state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: WaitArgs = parse_args(args)?;
    tokio::time::sleep(std::time::Duration::from_millis(args.ms)).await;
    Ok(json!({ "success": true }))
}

pub async fn clipboard_get(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    // The shared clipboard is global; display_id is accepted for interface
    // stability and documented as ignored.
    let _ = args;
    Ok(json!({ "text": state.runtime.clipboard().get() }))
}

pub async fn clipboard_set(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ClipboardSetArgs = parse_args(args)?;
    state.runtime.clipboard().set(&args.text);
    Ok(json!({ "success": true }))
}

pub async fn display_info(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DisplayIdArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let info = display.lock().await.info();
    Ok(json!({
        "display": info.display,
        "width": info.width,
        "height": info.height,
        "app_count": info.app_count,
    }))
}

pub async fn get_window_state(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: GetWindowStateArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let state = driver
        .get_window_state(
            args.pid as u32,
            args.window_id as u32,
            args.include_tree,
            args.include_screenshot,
        )
        .await?;

    let mut result = json!({
        "window_id": state.window_id,
        "title": state.title,
        "app_name": state.app_name,
        "bounds": {
            "x": state.bounds.x,
            "y": state.bounds.y,
            "w": state.bounds.w,
            "h": state.bounds.h,
        },
    });

    if let Some(tree) = state.tree {
        // Server-side budget: query filter first (keep matching elements),
        // then depth cap, then element count cap. Applied after the walk so
        // the AT-SPI semantics are untouched; the model pays only for what
        // it asked for.
        let query = args.query.as_ref().map(|q| q.to_lowercase());
        let matches_query = |role: &str, name: Option<&String>| match &query {
            Some(q) => {
                role.to_lowercase().contains(q)
                    || name.is_some_and(|n| n.to_lowercase().contains(q))
            }
            None => true,
        };
        let mut elements: Vec<Value> = tree
            .elements
            .iter()
            .filter(|e| matches_query(&e.role, e.name.as_ref()))
            .filter(|e| args.max_depth.is_none_or(|d| (e.depth as u32) <= d))
            .map(|e| {
                let mut node = json!({
                    "index": e.index,
                    "actionable": e.actionable,
                    "role": e.role,
                    "name": e.name,
                    "actions": e.actions,
                    "depth": e.depth,
                });
                if let Some(v) = &e.value {
                    node["value"] = json!(v);
                }
                if let Some(c) = e.checked {
                    node["checked"] = json!(c);
                }
                if let Some(en) = e.enabled {
                    node["enabled"] = json!(en);
                }
                if let Some(se) = e.selected {
                    node["selected"] = json!(se);
                }
                if let Some(d) = &e.description {
                    node["description"] = json!(d);
                }
                if let Some(p) = e.parent_index {
                    node["parent_index"] = json!(p);
                }
                if let Some(b) = &e.frame {
                    node["frame"] = json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h });
                }
                node
            })
            .collect();
        // Element-count cap after filtering; parents beyond the cap keep
        // their indexes valid (indexes are stable, links may be dropped).
        let total = elements.len();
        if let Some(max) = args.max_elements {
            elements.truncate(max as usize);
        }
        let truncated = args
            .max_elements
            .is_some_and(|max| total > max as usize);
        result["tree"] = json!({
            "elements": elements,
            "truncated": truncated,
            "elements_returned": elements.len(),
            "elements_total": total,
        });
    }

    if let Some(screenshot) = state.screenshot {
        let data = match args.max_image_dimension {
            Some(max_dim) => downscale_png(&screenshot.data, max_dim),
            None => encode_png(&screenshot.data),
        };
        result["screenshot"] = json!(data);
    }

    Ok(result)
}

pub async fn list_user_windows(_state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: ListUserWindowsArgs = parse_args(args)?;
    let windows = lgh_state::list_user_windows(args.name).await?;
    let nodes: Vec<Value> = windows
        .iter()
        .map(|w| {
            let mut node = json!({ "pid": w.pid, "process": w.process });
            if let Some(t) = &w.title {
                node["title"] = json!(t);
            }
            if let Some(b) = &w.bounds {
                node["bounds"] = json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h });
            }
            node
        })
        .collect();
    Ok(json!({ "windows": nodes, "count": nodes.len() }))
}

pub fn list_apps() -> Value {
    let apps: Vec<Value> = crate::apps::list_apps()
        .iter()
        .map(|a| {
            let mut node = json!({
                "name": a.name,
                "running": a.running,
            });
            if let Some(pid) = a.pid {
                node["pid"] = json!(pid);
            }
            if let Some(p) = &a.launch_path {
                node["launch_path"] = json!(p);
            }
            if let Some(b) = &a.bundle_id {
                node["bundle_id"] = json!(b);
            }
            node
        })
        .collect();
    json!({ "apps": apps, "count": apps.len() })
}

pub async fn get_desktop_overview(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: DesktopOverviewArgs = parse_args(args)?;
    let driver = find_driver(state, &args.display_id).await?;
    let overview = driver.get_desktop_overview().await?;

    let processes: Vec<Value> = overview
        .processes
        .iter()
        .map(|p| json!({ "pid": p.pid, "name": p.name }))
        .collect();
    let windows: Vec<Value> = overview
        .windows
        .iter()
        .filter(|w| args.pid.is_none_or(|pid| w.pid == Some(pid)))
        .filter(|w| !args.on_screen_only.unwrap_or(false) || w.on_screen)
        .map(|w| {
            let mut node = json!({
                "window_id": w.id,
                "pid": w.pid,
                "title": w.title,
                "z_index": w.z_index,
                "on_screen": w.on_screen,
            });
            if let Some(b) = &w.bounds {
                node["bounds"] = json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h });
            }
            node
        })
        .collect();

    Ok(json!({ "processes": processes, "windows": windows }))
}

pub async fn set_value(state: &DaemonState, args: &Value) -> Result<Value, LghError> {
    let args: SetValueArgs = parse_args(args)?;
    let display = find_display(state, &args.display_id).await?;
    let focus_allowed = !display.lock().await.is_external();
    let driver = find_driver(state, &args.display_id).await?;
    driver
        .set_value(args.pid as u32, args.index as usize, &args.value, focus_allowed)
        .await?;
    Ok(json!({ "success": true }))
}

pub fn tool_definitions() -> Vec<Value> {
    tools_tool_definitions()
}

async fn find_display(state: &DaemonState, id: &str) -> Result<Arc<Mutex<Display>>, LghError> {
    state
        .displays
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| LghError::DisplayNotFound(id.into()))
}

async fn find_driver(state: &DaemonState, id: &str) -> Result<Arc<dyn Driver>, LghError> {
    state
        .drivers
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| LghError::DisplayNotFound(id.into()))
}

fn encode_png(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}

/// Downscale a PNG so its longest edge is at most `max_dim` pixels
/// (never upscaled). Falls back to the original bytes on any decode
/// problem rather than failing the whole call.
fn downscale_png(data: &[u8], max_dim: u32) -> String {
    let decoder = match image::codecs::png::PngDecoder::new(std::io::Cursor::new(data)) {
        Ok(d) => d,
        Err(_) => return encode_png(data),
    };
    use image::{ImageDecoder, ImageEncoder};
    let (w, h) = decoder.dimensions();
    let long = w.max(h);
    if long <= max_dim || long == 0 {
        return encode_png(data);
    }
    let scale = max_dim as f64 / long as f64;
    let out_w = ((w as f64 * scale).round() as u32).max(1);
    let out_h = ((h as f64 * scale).round() as u32).max(1);
    let resized = (|| -> Option<Vec<u8>> {
        let mut buf = vec![0u8; decoder.total_bytes() as usize];
        decoder.read_image(&mut buf).ok()?;
        let img = image::RgbaImage::from_raw(w, h, buf)?;
        let scaled = image::imageops::resize(
            &img,
            out_w,
            out_h,
            image::imageops::FilterType::Triangle,
        );
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut png))
            .write_image(&scaled, out_w, out_h, image::ExtendedColorType::Rgba8)
            .ok()?;
        Some(png)
    })()
    .unwrap_or_else(|| data.to_vec());
    encode_png(&resized)
}
