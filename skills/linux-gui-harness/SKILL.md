---
name: linux-gui-harness
description: Run and automate Linux GUI applications in isolated X11 displays.
license: MIT
compatibility: Linux with xvfb and openbox installed.
---

# linux-gui-harness

Automate Linux GUI apps in isolated X11 displays (Xvfb + openbox) with a
live preview panel on the user's desktop — without touching their real
desktop.

## Decision flow for any GUI app request

Check whether the app already runs on the user's default display (`:0`)
with `lgh_list_user_windows` (optional `name` filter):

- **Already on `:0`** → focus it with `wmctrl -a <title>`; do NOT create
  a display.
- **Not running** → `lgh_display_create` + `lgh_app_launch`. NEVER launch
  GUI apps with shell commands on the user's desktop.
- **User says "automate", "test", "sandbox", "don't touch my desktop"** →
  always use this skill, even if the app runs on `:0`.

Shell is for non-GUI work and focusing existing windows — never for
launching GUI apps.

## Calling the tools

- **MCP agents**: the server is registered as `linux-gui-harness`; call
  `lgh_*` tools directly.
- **No MCP**: use the bundled helper next to this file:

  ```bash
  LGH_SOCKET_PATH=/tmp/lgh-opencode.sock \
    python3 <this-directory>/lgh.py lgh_display_create '{}'
  ```

  Never start your own Xvfb — always go through the daemon.

## Lifecycle

A display is bound to the agent session that created it — the session
id your host attaches to every `lgh_*` call. Destroy displays you no
longer need with `lgh_display_destroy`; leftovers are also reclaimed
when the session's MCP client disconnects, and hook-equipped hosts (pi,
opencode, codex) end sessions explicitly on /new, /fork or session
deletion. To hand a display to a forked session instead of losing it,
call `lgh_display_reown` with the new session id — only the current
owner can reown a display.

1. `lgh_list_displays` — see what already exists (skip create if reusable).
2. `lgh_display_create` — creates the display and the preview panel.
3. `lgh_app_launch` — start the app (returns pid).
4. `lgh_wait` / `lgh_get_desktop_overview` — wait for the window; the
   overview gives `window_id`, bounds and z-order (zombie windows are
   filtered).
5. Automate: `lgh_input_click/type/key`, `lgh_click_element` (AT-SPI
   doAction), `lgh_set_value`, `lgh_invoke_menu`, `lgh_verify_state`.
6. `lgh_capture_window` / `lgh_zoom` — screenshots (display-absolute
   coordinates; `window_id` identifies, not transforms).
7. `lgh_app_terminate` + `lgh_display_destroy` — clean up.

## Reading screen text without vision

If you cannot view images, or the app exposes no accessibility tree:

```bash
lgh_capture_window '{"display_id": "d-...", "window_id": 123,
                     "save_to": "/tmp/shot.png"}'
lgh_ocr '{"image_path": "/tmp/shot.png"}'
```

`save_to` writes the PNG and returns its path instead of base64 data;
`lgh_ocr` returns the recognized text (tesseract, else rapidocr).
`lgh_zoom` accepts the same `save_to` for small regions.

## Gotchas

- AT-SPI works for GTK/Qt/Electron apps; Chromium needs the a11y
  environment broadcast (the daemon sets it). Apps like WeChat expose
  nothing — use the OCR flow above.
- `lgh_click_element` and `lgh_set_value` accept an `identity` from a
  previous `lgh_get_window_state` for drift-proof targeting.
- `lgh_input_type` accepts an optional `pid` for an AT-SPI write fallback
  when the focused widget is not editable.
- `lgh_verify_state` is single-sample; call `lgh_wait` between attempts.

Full tool reference and JSON examples: `docs/usage.md` in the repository.
