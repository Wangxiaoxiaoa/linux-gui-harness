use lgh_core::LghError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    pub display_id: String,
    #[serde(default)]
    pub save_to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ButtonArg {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DisplayCreateArgs {
    #[serde(default)]
    pub preview: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDisplaysArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DisplayIdArgs {
    pub display_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReownArgs {
    pub display_id: String,
    /// Session id the display is transferred to (e.g. the forked session).
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DesktopOverviewArgs {
    pub display_id: String,
    /// Only return windows of this process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Only return windows that are mapped on screen. Default false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_screen_only: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListUserWindowsArgs {
    /// Case-insensitive substring filter on process name or window title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAppsArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InvokeMenuArgs {
    pub display_id: String,
    pub pid: u32,
    /// Menu path from the application menu bar, e.g. ["File", "Open"].
    /// Segments are matched case-sensitively after trimming whitespace.
    pub path: Vec<String>,
}

/// One state assertion. At least one of window/element should be present.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StateExpectationArgs {
    /// Window predicate for the pid's managed window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowExpectationArgs>,
    /// Element predicate against the AT-SPI tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ElementExpectationArgs>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WindowExpectationArgs {
    /// The window must exist. Note: absence cannot be proven reliably;
    /// only true is accepted.
    #[serde(default)]
    pub exists: Option<bool>,
    /// The window title must contain this string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ElementExpectationArgs {
    /// The element's AT-SPI role must equal this string (e.g. "button").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Some element's name must contain this string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_contains: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VerifyStateArgs {
    pub display_id: String,
    pub pid: u32,
    /// One to eight predicates, combined with logical AND.
    pub expect: Vec<StateExpectationArgs>,
    /// Bounded wait: keep sampling until the predicates hold or the \
    /// deadline passes. Zero performs a single sample.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Consecutive satisfied samples required before returning satisfied.
    #[serde(default)]
    pub stable_samples: Option<u64>,
    /// Attach a screenshot of the final window state as visual evidence.
    #[serde(default)]
    pub include_screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HoverArgs {
    pub display_id: String,
    pub x: f64,
    pub y: f64,
    /// How long to hold the mouse at (x, y). Default 1500 ms.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZoomArgs {
    pub display_id: String,
    pub window_id: u64,
    /// Region in window coordinates (screenshot pixels).
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// Save the PNG to this path and return the file path instead of base64
    /// data. Use with lgh_ocr when the model cannot view images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OcrArgs {
    /// Path of the image to read (e.g. saved by lgh_capture_window with
    /// `save_to`).
    pub image_path: String,
    /// Language hint for the engine (tesseract: e.g. "chi_sim+eng").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppLaunchArgs {
    pub display_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppTerminateArgs {
    pub display_id: String,
    pub pid: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWindowStateArgs {
    pub display_id: String,
    pub pid: u64,
    pub window_id: u64,
    #[serde(default)]
    pub include_tree: bool,
    #[serde(default)]
    pub include_screenshot: bool,
    /// Cap the number of returned tree elements (token budget for the \
    /// model). Elements beyond the cap are dropped with \
    /// truncated=true.
    #[serde(default)]
    pub max_elements: Option<u32>,
    /// Cap the tree depth.
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// Server-side filter: only elements whose role or name contains \
    /// this substring (case-insensitive) are returned.
    #[serde(default)]
    pub query: Option<String>,
    /// Downscale the screenshot so its longest edge is at most this \
    /// many pixels (fewer image tokens for multimodal callers).
    #[serde(default)]
    pub max_image_dimension: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickArgs {
    pub display_id: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: Option<ButtonArg>,
    #[serde(default)]
    pub count: Option<u64>,
    /// Interpret x/y as coordinates in the last lgh_zoom image instead of
    /// display coordinates. Requires a lgh_zoom call on this display first.
    #[serde(default)]
    pub from_zoom: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveArgs {
    pub display_id: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub from_zoom: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeArgs {
    pub display_id: String,
    pub text: String,
    /// Target process. Enables the AT-SPI write fallback when the focused
    /// widget is not editable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyArgs {
    pub display_id: String,
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollArgs {
    pub display_id: String,
    #[serde(default)]
    pub dx: i64,
    #[serde(default)]
    pub dy: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DragArgs {
    pub display_id: String,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// Modifier keys held for the whole gesture (e.g. shift for range
    /// selection, ctrl for copy-drag).
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub from_zoom: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WindowIdArgs {
    pub display_id: String,
    pub window_id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptureWindowArgs {
    pub display_id: String,
    pub window_id: u64,
    /// Save the PNG to this path and return the file path instead of base64
    /// data. Use with lgh_ocr when the model cannot view images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetWindowFrameArgs {
    pub display_id: String,
    pub window_id: u64,
    pub x: i64,
    pub y: i64,
    pub width: u64,
    pub height: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetValueArgs {
    pub display_id: String,
    pub pid: u64,
    pub index: u64,
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickElementArgs {
    pub display_id: String,
    pub pid: u64,
    pub index: u64,
    #[serde(default)]
    pub button: Option<ButtonArg>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitArgs {
    pub ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClipboardSetArgs {
    pub display_id: String,
    pub text: String,
}

/// JSON Schema for a tool's arguments, tuned for MCP clients:
/// subschemas are inlined (no `$ref`/`definitions`) and `Option<T>`
/// does not become a `null` union, because several clients (e.g.
/// gemini-cli-based agents) reject or skip schemas they cannot flatten.
fn root_schema<T: JsonSchema>() -> Value {
    let settings = schemars::gen::SchemaSettings::draft07().with(|settings| {
        settings.inline_subschemas = true;
        settings.option_add_null_type = false;
        settings.option_nullable = false;
    });
    let mut schema =
        serde_json::to_value(settings.into_generator().into_root_schema_for::<T>()).unwrap();
    // schemars emits e.g. "format": "uint64"; some MCP clients reject unknown
    // formats, and no tool relies on format semantics.
    strip_format(&mut schema);
    schema
}

fn strip_format(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("format");
            for child in map.values_mut() {
                strip_format(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_format(child);
            }
        }
        _ => {}
    }
}

fn tool_def(name: &str, description: &str, schema: Value) -> Value {
    let mut schema = schema;
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": annotations_for(name),
    })
}

/// MCP behavior hints per tool. Every tool operates on harness displays the
/// daemon owns, so the world stays closed.
fn annotations_for(name: &str) -> Value {
    let read_only = matches!(
        name,
        "lgh_display_info"
            | "lgh_list_apps"
            | "lgh_list_user_windows"
            | "lgh_get_desktop_overview"
            | "lgh_get_window_state"
            | "lgh_capture_window"
            | "lgh_screenshot"
            | "lgh_ocr"
            | "lgh_zoom"
            | "lgh_clipboard_get"
            | "lgh_input_get_cursor_position"
            | "lgh_wait"
            | "lgh_verify_state"
    );
    let destructive = matches!(
        name,
        "lgh_display_destroy" | "lgh_app_terminate" | "lgh_window_close"
    );
    let idempotent = matches!(
        name,
        "lgh_display_info"
            | "lgh_list_apps"
            | "lgh_list_user_windows"
            | "lgh_get_desktop_overview"
            | "lgh_get_window_state"
            | "lgh_capture_window"
            | "lgh_ocr"
            | "lgh_zoom"
            | "lgh_clipboard_get"
            | "lgh_clipboard_set"
            | "lgh_input_get_cursor_position"
            | "lgh_input_move"
            | "lgh_wait"
            | "lgh_verify_state"
            | "lgh_set_value"
            | "lgh_window_focus"
            | "lgh_window_set_frame"
            | "lgh_preview_open"
            | "lgh_preview_close"
            | "lgh_display_attach"
            | "lgh_display_detach"
    );
    json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false,
    })
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "lgh_display_create",
            "Create an isolated X11 display with a live preview panel on the \
             user's desktop. Routing rule for GUI app requests: first call \
             lgh_list_user_windows — if the app already runs on the user's \
             desktop (:0), focus it with `wmctrl -a` instead of calling this; \
             if it does NOT run there, ALWAYS open it through this tool plus \
             lgh_app_launch, never by shell-launching GUI apps on the user's \
             desktop.",
            root_schema::<DisplayCreateArgs>(),
        ),
        tool_def(
            "lgh_display_destroy",
            "Destroy an X11 display",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_list_displays",
            "List all harness displays. Each entry includes display_id, display \
             number, size and running app count. Displays are destroyed when \
             the session that created them ends; create a new one with \
             lgh_display_create if none fits.",
            root_schema::<ListDisplaysArgs>(),
        ),
        tool_def(
            "lgh_display_attach",
            "Attach to an existing X11 display (e.g. :0)",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_display_detach",
            "Detach from an existing X11 display without destroying it",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_app_launch",
            "Launch a GUI app inside a harness display (created with \
             lgh_display_create). Never launch GUI apps with shell commands \
             on the user's desktop — GUI app requests go through this tool \
             so they run in the sandbox with a live preview.",
            root_schema::<AppLaunchArgs>(),
        ),
        tool_def(
            "lgh_app_terminate",
            "Terminate an application by PID",
            root_schema::<AppTerminateArgs>(),
        ),
        tool_def(
            "lgh_get_window_state",
            "Get window state: metadata, optional AT-SPI tree, optional screenshot",
            root_schema::<GetWindowStateArgs>(),
        ),
        tool_def(
            "lgh_input_click",
            "Click at screen coordinates. Supports left/right/middle buttons and multiple clicks.",
            root_schema::<ClickArgs>(),
        ),
        tool_def(
            "lgh_input_move",
            "Move the mouse cursor",
            root_schema::<MoveArgs>(),
        ),
        tool_def(
            "lgh_input_type",
            "Type text via XTEST. Accepts optional pid: when the focused widget is not \
             editable after XTEST delivery, the text is written through the pid's \
             AT-SPI tree instead of vanishing.",
            root_schema::<TypeArgs>(),
        ),
        tool_def(
            "lgh_input_key",
            "Press a key or key combination. Key names (case-insensitive): \
             enter/return, esc/escape, tab, space, backspace, del/delete, \
             ins/insert, home, end, pgup/pageup, pgdn/pagedown, \
             up/down/left/right, f1-f12; modifiers: ctrl, alt, shift, super; \
             or an exact X keysym name, or a single character. Unknown \
             multi-character names are rejected (never typed as text).",
            root_schema::<KeyArgs>(),
        ),
        tool_def(
            "lgh_input_scroll",
            "Scroll by a delta",
            root_schema::<ScrollArgs>(),
        ),
        tool_def(
            "lgh_input_drag",
            "Drag from (x1, y1) to (x2, y2)",
            root_schema::<DragArgs>(),
        ),
        tool_def(
            "lgh_input_get_cursor_position",
            "Get the current mouse cursor position",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_hover",
            "Move the mouse to (x, y), hold for duration_ms (default 1500 ms), then return \
             a screenshot of the window under the cursor (or the full display if the \
             cursor is on the desktop). Use this to reveal tooltips, hover states, or \
             context hints when the next action is unclear.",
            root_schema::<HoverArgs>(),
        ),
        tool_def(
            "lgh_zoom",
            "Capture a cropped image of a region (x1,y1)-(x2,y2) in display coordinates (the \
             same space as a11y frames and input coordinates), padded by 20% and scaled to \
             at most 500 px wide. The result includes display_origin and scale; pass \
             from_zoom=true to lgh_input_click / lgh_input_move / lgh_input_drag to \
             translate coordinates from the zoom image back to display space.",
            root_schema::<ZoomArgs>(),
        ),
        tool_def(
            "lgh_capture_window",
            "Take a screenshot of a specific window as PNG. By default returns \
             base64 data; with `save_to` writes the file instead and returns \
             its path — pair with lgh_ocr when the model cannot view images.",
            root_schema::<CaptureWindowArgs>(),
        ),
        tool_def(
            "lgh_ocr",
            "Read text from an image file with a local OCR engine (tesseract, \
             else rapidocr). Pair with lgh_capture_window/lgh_zoom `save_to` to \
             read screen text when the model has no vision or the app exposes \
             no accessibility tree. Returns the recognized text and the engine \
             used.",
            root_schema::<OcrArgs>(),
        ),
        tool_def(
            "lgh_list_apps",
            "List applications: running processes merged with installed XDG desktop \
             entries. Each entry includes name, running (with pid when live), launch_path \
             (pass to lgh_app_launch) and bundle_id. Use it to answer \"is X installed?\" \
             and \"is X running?\".",
            root_schema::<ListAppsArgs>(),
        ),
        tool_def(
            "lgh_invoke_menu",
            "Resolve an application menu path (e.g. [\"File\", \"Open\"]) through AT-SPI \
             and invoke the final item. Path segments are matched case-sensitively \
             against direct children of the menu bar; fails with the exact segment \
             when not found.",
            root_schema::<InvokeMenuArgs>(),
        ),
        tool_def(
            "lgh_verify_state",
            "Deterministically verify UI state: window (exists, title_contains) \
             and AT-SPI element (role, label_contains) predicates, combined with \
             logical AND. Returns per-predicate outcomes with the observed state \
             and a three-state verdict (satisfied/unsatisfied/unknown). \
             UI changes are asynchronous: pass timeout_ms to keep sampling until \
             the predicates hold (e.g. after clicking a button that opens a \
             dialog), and stable_samples to require the state to hold across \
             consecutive samples. include_screenshot attaches the final window \
             screenshot as visual evidence for multimodal callers.",
            root_schema::<VerifyStateArgs>(),
        ),
        tool_def(
            "lgh_get_desktop_overview",
            "Return desktop overview: running processes and windows. Stale windows from \
             exited processes are never returned. Each window record includes window_id, pid, \
             title, bounds, z_index (higher = closer to front) and on_screen.",
            root_schema::<DesktopOverviewArgs>(),
        ),
        tool_def(
            "lgh_list_user_windows",
            "List GUI windows currently running on the user's default desktop (:0) with \
             pid, process name, title and geometry. Routing rule: call this BEFORE \
             lgh_display_create for any GUI app request — if the app already runs on \
             the user's desktop, focus it with `wmctrl -a` instead of creating a display; \
             if it does not, create a harness display and launch it there. Optional \
             `name` filters by process-name or title substring (case-insensitive).",
            root_schema::<ListUserWindowsArgs>(),
        ),
        tool_def(
            "lgh_set_value",
            "Set the value of an AT-SPI editable element. Accepts optional identity \
             (from a previous lgh_get_window_state) for drift-proof targeting. \
             Uses EditableText.SetTextContents with a clear-then-insert fallback. \
             On harness displays the target widget is focused first when the \
             toolkit requires it; attached external displays are never raised.",
            root_schema::<SetValueArgs>(),
        ),
        tool_def(
            "lgh_click_element",
            "Activate an AT-SPI element by pid and index. Accepts optional identity \
             (from a previous lgh_get_window_state) for drift-proof targeting. \
             Left-click uses AT-SPI doAction (focus-free, works on background \
             windows); passive elements return suspected_noop.",
            root_schema::<ClickElementArgs>(),
        ),
        tool_def(
            "lgh_wait",
            "Wait for a number of milliseconds",
            root_schema::<WaitArgs>(),
        ),
        tool_def(
            "lgh_window_focus",
            "Focus a window by id",
            root_schema::<WindowIdArgs>(),
        ),
        tool_def(
            "lgh_window_set_frame",
            "Set a window's position and size",
            root_schema::<SetWindowFrameArgs>(),
        ),
        tool_def(
            "lgh_window_close",
            "Close a window by id",
            root_schema::<WindowIdArgs>(),
        ),
        tool_def(
            "lgh_clipboard_get",
            "Get text from the clipboard",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_clipboard_set",
            "Set text on the clipboard",
            root_schema::<ClipboardSetArgs>(),
        ),
        tool_def(
            "lgh_display_info",
            "Get display metadata",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_preview_open",
            "Open a preview window for a display if not already open",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_preview_close",
            "Close the preview window for a display",
            root_schema::<DisplayIdArgs>(),
        ),
        tool_def(
            "lgh_session_end",
            "End the current agent session's displays: destroy every harness \
             display this session created. Call when the task is fully done \
             and the user confirmed nothing else is needed; displays are \
             also destroyed automatically when the session disconnects.",
            root_schema::<ListDisplaysArgs>(),
        ),
        tool_def(
            "lgh_display_reown",
            "Transfer a display you own to another session id (e.g. hand it \
             to a forked session so it can keep working with the same GUI). \
             Only the current owner can reown a display.",
            root_schema::<ReownArgs>(),
        ),
    ]
}

pub fn parse_args<T: for<'de> Deserialize<'de>>(args: &Value) -> Result<T, LghError> {
    serde_json::from_value(args.clone())
        .map_err(|e| LghError::InvalidArgument(format!("invalid arguments: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_definitions_has_expected_tools() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 37, "expected 37 tool definitions");

        let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"lgh_display_create"));
        assert!(names.contains(&"lgh_display_reown"));
        assert!(names.contains(&"lgh_session_end"));
        assert!(names.contains(&"lgh_list_displays"));
        assert!(names.contains(&"lgh_list_apps"));
        assert!(names.contains(&"lgh_list_user_windows"));
        assert!(names.contains(&"lgh_zoom"));
        assert!(names.contains(&"lgh_invoke_menu"));
        assert!(names.contains(&"lgh_verify_state"));
        assert!(names.contains(&"lgh_input_click"));
        assert!(names.contains(&"lgh_clipboard_get"));
        assert!(names.contains(&"lgh_window_set_frame"));
        assert!(names.contains(&"lgh_preview_open"));
        assert!(names.contains(&"lgh_preview_close"));
    }

    #[test]
    fn tool_definitions_have_schema() {
        for def in tool_definitions() {
            assert!(def["name"].is_string(), "tool name missing: {def}");
            assert!(
                def["description"].is_string(),
                "tool description missing: {def}"
            );
            assert!(
                def["inputSchema"].is_object(),
                "tool inputSchema missing: {def}"
            );
        }
    }

    #[test]
    fn parse_display_create_args() {
        let args = json!({"preview": false, "name": "demo"});
        let parsed = parse_args::<DisplayCreateArgs>(&args).unwrap();
        assert_eq!(parsed.preview, Some(false));
        assert_eq!(parsed.name.as_deref(), Some("demo"));

        let default = parse_args::<DisplayCreateArgs>(&json!({})).unwrap();
        assert_eq!(default.preview, None);
        assert_eq!(default.name, None);
    }

    #[test]
    fn parse_click_args_with_button() {
        let args = json!({"display_id": "d-1", "x": 10, "y": 20, "button": "right", "count": 2});
        let parsed = parse_args::<ClickArgs>(&args).unwrap();
        assert_eq!(parsed.x, 10.0);
        assert_eq!(parsed.y, 20.0);
        assert!(matches!(parsed.button, Some(ButtonArg::Right)));
        assert_eq!(parsed.count, Some(2));
    }

    #[test]
    fn parse_missing_required_field_is_rejected() {
        let args = json!({"display_id": "d-1"}); // missing command
        let err = parse_args::<AppLaunchArgs>(&args).unwrap_err();
        assert!(matches!(err, LghError::InvalidArgument(_)));
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn schemas_are_client_friendly() {
        for def in tool_definitions() {
            let schema = &def["inputSchema"];
            let raw = serde_json::to_string(schema).unwrap();
            assert!(!raw.contains("\"$ref\""), "{} has $ref: {raw}", def["name"]);
            assert!(
                !raw.contains("definitions"),
                "{} has definitions",
                def["name"]
            );
            assert!(!raw.contains("format"), "{} has format: {raw}", def["name"]);
            let Some(props) = schema["properties"].as_object() else {
                // Parameter-less tools (e.g. lgh_list_apps) have no properties.
                continue;
            };
            for (name, prop) in props {
                assert!(
                    prop.get("type").is_some(),
                    "{}.{} has no type: {prop}",
                    def["name"],
                    name
                );
            }
        }
    }
}
