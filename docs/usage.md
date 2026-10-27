# Usage

## Build

```bash
cargo build --release
```

The MCP server binary is `target/release/linux-gui-harness`.

## Run the MCP server

The MCP server is a long-lived daemon plus a per-agent stdio proxy.

```bash
./target/release/linux-gui-harness serve  # start daemon
./target/release/linux-gui-harness mcp    # stdio proxy (auto-starts daemon)
./target/release/linux-gui-harness status # check daemon
./target/release/linux-gui-harness stop   # stop daemon
```

## Multiple isolated instances

By default, all agents share one daemon and each MCP connection gets its own session. Displays created by one session are cleaned up when that session disconnects.

To run a fully separate daemon process, use a unique socket:

```bash
./target/release/linux-gui-harness serve --socket /tmp/lgh-agent-1.sock
./target/release/linux-gui-harness mcp --socket /tmp/lgh-agent-1.sock
./target/release/linux-gui-harness stop --socket /tmp/lgh-agent-1.sock
```

`--socket` overrides the default socket path (`$XDG_RUNTIME_DIR/linux-gui-harness.sock`, falling back to `/tmp/linux-gui-harness.sock`) and the `LGH_SOCKET_PATH` environment variable.

## Display preview

Displays created on this machine automatically open a live preview panel on your desktop, scaled to a fraction of your screen (aspect ratio preserved). The preview is rendered by the daemon and never touches the display's lifecycle.

- Previews are **read-only by default** — safe to keep on screen while the agent works.
- **Double-click a preview** to open an expanded interactive window (~50% screen height, docked top-left): your mouse clicks, motion, wheel and keyboard are forwarded into the sandboxed app via XTEST, and the view refreshes ~100ms. Exit with `Esc` or the window's close button; the small thumbnail keeps rendering in the panel.
- Only one expanded window exists at a time; double-clicking the same cell toggles it closed, another cell replaces it.
- Closing the preview panel (or calling `lgh_preview_close`) does not affect the display.
- Reopen it later with `lgh_preview_open`.
- The preview requires a running desktop session (`DISPLAY`). On headless hosts no preview is created and everything else works as usual.

## Routing: check the user's desktop first

For any GUI app request, call `lgh_list_user_windows` (optionally filtered by `name`) before creating a display: if the app already runs on the user's desktop (`:0`), focus it with `wmctrl -a` instead of creating a sandbox; if not, create one with `lgh_display_create` + `lgh_app_launch`.

## Reading screen text without vision

If the model cannot view images, or the app exposes no accessibility tree, capture with `save_to` and read with `lgh_ocr`:

```json
{"method":"tools/call","params":{"name":"lgh_capture_window","arguments":{"display_id":"d-1","window_id":123,"save_to":"/tmp/shot.png"}}}
{"method":"tools/call","params":{"name":"lgh_ocr","arguments":{"image_path":"/tmp/shot.png"}}}
```

`save_to` writes the PNG and returns its path instead of base64 data (also available on `lgh_zoom`). `lgh_ocr` probes engines in order: tesseract, then the python package `rapidocr_onnxruntime`.

## Example MCP session

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"lgh_display_create","arguments":{}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"lgh_app_launch","arguments":{"display_id":"d-99","command":"xterm","args":[]}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"lgh_get_desktop_overview","arguments":{"display_id":"d-99"}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"lgh_capture_window","arguments":{"display_id":"d-99","window_id":4194314,"save_to":"/tmp/shot.png"}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"lgh_ocr","arguments":{"image_path":"/tmp/shot.png"}}}
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"lgh_display_destroy","arguments":{"display_id":"d-99"}}}
```

## Tool categories

| Category | Tools |
|---|---|
| Display | `lgh_display_create` `lgh_display_destroy` `lgh_display_attach` `lgh_display_detach` `lgh_display_info` |
| Apps | `lgh_app_launch` `lgh_app_terminate` `lgh_list_apps` |
| User desktop | `lgh_list_user_windows` |
| Input | `lgh_input_click` `lgh_input_move` `lgh_input_type` `lgh_input_key` `lgh_input_scroll` `lgh_input_drag` `lgh_input_get_cursor_position` `lgh_hover` |
| Capture | `lgh_capture_window` `lgh_zoom` |
| OCR | `lgh_ocr` |
| State | `lgh_get_desktop_overview` `lgh_get_window_state` `lgh_verify_state` |
| AT-SPI | `lgh_set_value` `lgh_click_element` `lgh_invoke_menu` |
| Window | `lgh_window_focus` `lgh_window_set_frame` `lgh_window_close` |
| Clipboard | `lgh_clipboard_get` `lgh_clipboard_set` |
| Preview | `lgh_preview_open` `lgh_preview_close` |
| Wait | `lgh_wait` |

Notable parameters:

- `lgh_input_click` / `lgh_input_move` / `lgh_input_drag` accept `from_zoom: true` to interpret coordinates in the last `lgh_zoom` image instead of display coordinates.
- `lgh_input_type` accepts an optional `pid` to enable AT-SPI text writing when the focused widget is not editable.
- `lgh_get_window_state` accepts budgets for large applications: `max_elements`,
  `max_depth`, `query` (server-side role/name substring filter) and
  `max_image_dimension` (screenshot downscale). Truncation is reported as
  `truncated: true` with `elements_total`.
- `lgh_input_drag` accepts `modifiers` (e.g. ["shift"]) held for the whole gesture.
- `lgh_click_element` / `lgh_set_value` accept an optional `identity` string (from `lgh_get_window_state` elements) for drift-proof targeting.
- `lgh_get_desktop_overview` accepts `pid` and `on_screen_only` filters.

## Example scenarios

### Desktop app functional test

```json
{"method":"tools/call","params":{"name":"lgh_display_create","arguments":{}}}
{"method":"tools/call","params":{"name":"lgh_app_launch","arguments":{"display_id":"d-1","command":"gedit"}}}
{"method":"tools/call","params":{"name":"lgh_input_type","arguments":{"display_id":"d-1","text":"Hello World"}}}
{"method":"tools/call","params":{"name":"lgh_input_key","arguments":{"display_id":"d-1","key":"ctrl+s"}}}
// key names (case-insensitive): enter/return, esc/escape, tab, space, backspace, del/delete,
// ins/insert, home, end, pgup/pageup, pgdn/pagedown, up/down/left/right, f1-f12,
// or an exact X keysym name, or a single character. Unknown multi-character
// names are rejected (never typed as text).
{"method":"tools/call","params":{"name":"lgh_verify_state","arguments":{"display_id":"d-1","pid":1234,
  "expect":[{"window":{"exists":true,"title_contains":"Hello World"}}],
  "timeout_ms":15000,"stable_samples":2,"include_screenshot":true}}}
// UI changes are asynchronous: timeout_ms keeps sampling until the predicates hold,
// stable_samples requires the state to hold across consecutive samples, and
// include_screenshot attaches the final screenshot. Verdict:
// satisfied / unsatisfied / unknown(+reason).
{"method":"tools/call","params":{"name":"lgh_display_destroy","arguments":{"display_id":"d-1"}}}
```

### Browser automation (Chromium/Electron)

The daemon advertises accessibility on the session bus at startup, so Chromium and Electron apps render full AT-SPI trees. Launch with a URL, wait for the page to load, then use `lgh_get_window_state` to read links, buttons, and form fields from the tree.

### Menu invocation

```json
{"method":"tools/call","params":{"name":"lgh_invoke_menu","arguments":{"display_id":"d-1","pid":1234,"path":["File","Export as PDF"]}}}
```

### UI exploration with hover

When the agent encounters an unfamiliar button or icon, `lgh_hover` moves the mouse there, waits for tooltips to appear, and returns a screenshot for the agent to read.

### Zoom into fine detail

`lgh_zoom` captures a cropped, scaled region of a window. The response includes `display_origin` and `scale`; pass `from_zoom: true` to input tools to translate coordinates from the zoom image back to display space.

## Run tests

```bash
cargo test -- --test-threads=1
```

Integration tests live in `lgh/daemon/tests/integration.rs`. They exercise the daemon end-to-end and must run sequentially because each test allocates X11 displays.
