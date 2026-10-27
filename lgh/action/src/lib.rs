#![forbid(unsafe_code)]
use async_trait::async_trait;
use lgh_core::{x11, InputDriver, LghError, MouseButton};
use std::{thread, time::Duration};
use tokio::task;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT,
    KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// Blocking XTEST helpers shared by the async driver and the interactive
/// preview expando. Every function operates on an already-open connection
/// to the target display; none of them require a tokio runtime.
pub mod raw {
    use super::*;
    use x11rb::protocol::xproto::{GetKeyboardMappingReply, Window};

    /// Round-trip to the X server so every buffered request (key/button
    /// transitions) is actually delivered before this short-lived connection
    /// closes. X11 requests are client-buffered: without the round trip the
    /// server may process a press and its release back to back, erasing any
    /// intended hold interval, and a closing connection loses requests that
    /// are still in flight.
    pub fn deliver(conn: &RustConnection) -> Result<(), LghError> {
        conn.flush().map_err(x11::xerr)?;
        conn.get_input_focus()
            .map_err(x11::xerr)?
            .reply()
            .map_err(x11::xerr)?;
        Ok(())
    }

    pub fn get_mapping(conn: &RustConnection) -> Result<GetKeyboardMappingReply, LghError> {
        conn.get_keyboard_mapping(8, 248)
            .map_err(x11::xerr)?
            .reply()
            .map_err(x11::xerr)
    }

    pub fn keysym_for_char(ch: char) -> Option<u32> {
        x11_keysymdef::lookup_by_codepoint(ch).map(|r| r.keysym)
    }

    pub fn keysym_for_name(name: &str) -> Option<u32> {
        x11_keysymdef::lookup_by_name(name).map(|r| r.keysym)
    }

    /// Documented vocabulary for `lgh_input_key` (see the tool description):
    /// enter/return, esc/escape, tab, space, backspace, del/delete,
    /// ins/insert, home, end, pgup/pageup, pgdn/pagedown, arrows, f1-f12,
    /// and modifier names. Case-insensitive; returns the canonical X
    /// keysym name to look up.
    pub fn keysym_name_for(key: &str) -> Option<String> {
        let k = key.trim();
        if k.is_empty() {
            return None;
        }
        // f1-f12 (also F1..F12 as typed)
        if let Some(rest) = k.strip_prefix(['f', 'F']) {
            if let Ok(n) = rest.parse::<u8>() {
                if (1..=12).contains(&n) {
                    return Some(format!("F{n}"));
                }
            }
        }
        let canonical = match k.to_ascii_lowercase().as_str() {
            "enter" | "return" => "Return",
            "esc" | "escape" => "Escape",
            "tab" => "Tab",
            "space" => "space",
            "backspace" => "BackSpace",
            "del" | "delete" => "Delete",
            "ins" | "insert" => "Insert",
            "home" => "Home",
            "end" => "End",
            "pgup" | "pageup" => "Prior",
            "pgdn" | "pagedown" => "Next",
            "up" => "Up",
            "down" => "Down",
            "left" => "Left",
            "right" => "Right",
            "ctrl" | "control" => "Control_L",
            "alt" => "Alt_L",
            "shift" => "Shift_L",
            "super" | "win" => "Super_L",
            // keep the original case: exact X keysym names (Page_Down, A,
            // F1) are case-sensitive in the lookup
            _ => return Some(k.to_string()),
        };
        Some(canonical.to_string())
    }

    pub const KEY_VOCABULARY: &str = "enter/return, esc/escape, tab, space, backspace, del/delete, \
ins/insert, home, end, pgup/pageup, pgdn/pagedown, up/down/left/right, f1-f12, \
or an exact X keysym name, or a single character";

    /// The keysym produced by `keycode` at `level` (0 = base, 1 = shifted).
    pub fn keysym_at(mapping: &GetKeyboardMappingReply, keycode: u8, level: usize) -> Option<u32> {
        let per = mapping.keysyms_per_keycode as usize;
        if per == 0 || keycode < 8 {
            return None;
        }
        let idx = (keycode as usize - 8) * per + level;
        match mapping.keysyms.get(idx) {
            Some(&0) | None => None,
            Some(&keysym) => Some(keysym),
        }
    }

    pub fn keycode_for_keysym(
        mapping: &GetKeyboardMappingReply,
        keysym: u32,
    ) -> Option<(u8, bool)> {
        let per = mapping.keysyms_per_keycode as usize;
        if per == 0 {
            return None;
        }
        for (i, syms) in mapping.keysyms.chunks(per).enumerate() {
            if syms.first() == Some(&keysym) {
                return Some(((8 + i) as u8, false));
            }
            if per > 1 && syms.get(1) == Some(&keysym) {
                return Some(((8 + i) as u8, true));
            }
        }
        None
    }

    pub fn press(conn: &RustConnection, keycode: u8, down: bool) -> Result<(), LghError> {
        let event = if down {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        conn.xtest_fake_input(event, keycode, 0, x11rb::NONE, 0, 0, 0)
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)
    }

    pub fn mouse_button_number(button: MouseButton) -> u8 {
        match button {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 3,
        }
    }

    /// Warp the pointer to `(x, y)` and press or release one button there.
    pub fn button(
        conn: &RustConnection,
        root: Window,
        x: i32,
        y: i32,
        button: u8,
        down: bool,
    ) -> Result<(), LghError> {
        let event = if down {
            BUTTON_PRESS_EVENT
        } else {
            BUTTON_RELEASE_EVENT
        };
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x as i16, y as i16)
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)?;
        conn.xtest_fake_input(event, button, 0, root, x as i16, y as i16, 0)
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)?;
        deliver(conn)
    }

    /// Warp to `(x, y)` and click `count` times.
    pub fn click(
        conn: &RustConnection,
        root: Window,
        x: i32,
        y: i32,
        which: MouseButton,
        count: u32,
    ) -> Result<(), LghError> {
        let btn = mouse_button_number(which);
        for i in 0..count.max(1) {
            if i > 0 {
                thread::sleep(Duration::from_millis(50));
            }
            button(conn, root, x, y, btn, true)?;
            button(conn, root, x, y, btn, false)?;
        }
        Ok(())
    }

    /// Warp the pointer to `(x, y)`.
    pub fn move_to(conn: &RustConnection, root: Window, x: i32, y: i32) -> Result<(), LghError> {
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x as i16, y as i16)
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)?;
        deliver(conn)
    }

    /// Scroll by `(dx, dy)` wheel steps at the current pointer position.
    pub fn wheel(conn: &RustConnection, root: Window, dx: i32, dy: i32) -> Result<(), LghError> {
        let click = |button: u8| -> Result<(), LghError> {
            conn.xtest_fake_input(BUTTON_PRESS_EVENT, button, 0, root, 0, 0, 0)
                .map_err(x11::xerr)?
                .check()
                .map_err(x11::xerr)?;
            conn.xtest_fake_input(BUTTON_RELEASE_EVENT, button, 0, root, 0, 0, 0)
                .map_err(x11::xerr)?
                .check()
                .map_err(x11::xerr)?;
            Ok(())
        };

        for _ in 0..dy.abs() {
            click(if dy > 0 { 5 } else { 4 })?;
        }
        for _ in 0..dx.abs() {
            click(if dx > 0 { 7 } else { 6 })?;
        }
        deliver(conn)
    }
}

pub struct XtestInput {
    display: String,
}

impl XtestInput {
    pub fn new(display: &str) -> Result<Self, LghError> {
        let _ = x11::open_connection(display)?;
        Ok(Self {
            display: display.to_string(),
        })
    }
}

fn has_net_wm_pid(conn: &RustConnection, window: u32) -> bool {
    let atom = match conn.intern_atom(false, b"_NET_WM_PID") {
        Ok(c) => match c.reply() {
            Ok(r) => r.atom,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    let cardinal = match conn.intern_atom(false, b"CARDINAL") {
        Ok(c) => match c.reply() {
            Ok(r) => r.atom,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    conn.get_property(false, window, atom, cardinal, 0, 1)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.format == 32 && r.value.len() >= 4)
        .unwrap_or(false)
}

fn client_window(conn: &RustConnection, window: u32) -> u32 {
    if has_net_wm_pid(conn, window) {
        return window;
    }
    if let Ok(tree) = conn.query_tree(window).map_err(x11::xerr) {
        if let Ok(reply) = tree.reply() {
            for &child in &reply.children {
                if has_net_wm_pid(conn, child) {
                    return child;
                }
            }
        }
    }
    window
}

#[async_trait]
impl InputDriver for XtestInput {
    async fn click(&self, x: i32, y: i32, button: MouseButton, count: u32) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);
            raw::click(&conn, root, x, y, button, count)?;
            raw::deliver(&conn)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn move_mouse(&self, x: i32, y: i32) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);
            raw::move_to(&conn, root, x, y)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn scroll(&self, dx: i32, dy: i32) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);
            raw::wheel(&conn, root, dx, dy)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn drag(
        &self,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        modifiers: &[&str],
    ) -> Result<(), LghError> {
        let display = self.display.clone();
        let modifiers: Vec<String> = modifiers.iter().map(|s| s.to_string()).collect();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);

            // Resolve modifier keycodes up front so a bad name fails before
            // any button event reaches the display.
            let mapping = raw::get_mapping(&conn)?;
            let mut modifier_codes = Vec::new();
            for m in &modifiers {
                let name = raw::keysym_name_for(m).ok_or_else(|| {
                    LghError::InvalidArgument(format!("unknown modifier: {m}"))
                })?;
                let keysym = raw::keysym_for_name(&name).ok_or_else(|| {
                    LghError::InvalidArgument(format!("unknown modifier: {m}"))
                })?;
                let (code, _) = raw::keycode_for_keysym(&mapping, keysym)
                    .ok_or_else(|| {
                        LghError::InvalidArgument(format!("no keycode for modifier: {m}"))
                    })?;
                modifier_codes.push(code);
            }

            raw::move_to(&conn, root, x1, y1)?;
            // Hold modifiers for the whole gesture; release in reverse even
            // when a later step fails, so nothing strands on the display.
            for &code in &modifier_codes {
                raw::press(&conn, code, true)?;
            }
            let press_result = raw::button(&conn, root, x1, y1, 1, true);
            if let Err(e) = press_result {
                for &code in modifier_codes.iter().rev() {
                    let _ = raw::press(&conn, code, false);
                }
                return Err(e);
            }
            let motion = conn
                .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x2 as i16, y2 as i16, 0)
                .map_err(x11::xerr)?
                .check()
                .map_err(x11::xerr);
            if let Err(e) = motion {
                for &code in modifier_codes.iter().rev() {
                    let _ = raw::press(&conn, code, false);
                }
                return Err(e);
            }
            let release = raw::button(&conn, root, x2, y2, 1, false);
            for &code in modifier_codes.iter().rev() {
                let _ = raw::press(&conn, code, false);
            }
            release?;

            raw::deliver(&conn)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn type_text(&self, _pid: Option<u32>, text: &str) -> Result<(), LghError> {
        let display = self.display.clone();
        let text = text.to_string();
        task::spawn_blocking(move || {
            let (conn, _) = x11::open_connection(&display)?;
            let mapping = raw::get_mapping(&conn)?;

            for ch in text.chars() {
                let keysym = raw::keysym_for_char(ch).ok_or_else(|| {
                    LghError::InvalidArgument(format!("unknown character: {}", ch))
                })?;
                let (keycode, needs_shift) =
                    raw::keycode_for_keysym(&mapping, keysym).ok_or_else(|| {
                        LghError::InvalidArgument(format!("no keycode for keysym 0x{:x}", keysym))
                    })?;

                if needs_shift {
                    let (shift_kc, _) = raw::keycode_for_keysym(&mapping, 0xffe1)
                        .ok_or_else(|| LghError::InvalidArgument("no Shift keycode".into()))?;
                    raw::press(&conn, shift_kc, true)?;
                    raw::press(&conn, keycode, true)?;
                    raw::press(&conn, keycode, false)?;
                    raw::press(&conn, shift_kc, false)?;
                } else {
                    raw::press(&conn, keycode, true)?;
                    raw::press(&conn, keycode, false)?;
                }
            }

            raw::deliver(&conn)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn key(&self, key: &str, modifiers: &[&str]) -> Result<(), LghError> {
        let display = self.display.clone();
        let key = key.to_string();
        let modifiers: Vec<String> = modifiers.iter().map(|s| s.to_string()).collect();
        task::spawn_blocking(move || {
            let (conn, _) = x11::open_connection(&display)?;
            let mapping = raw::get_mapping(&conn)?;

            // Documented vocabulary first; an exact X keysym name next; a
            // single character last. Unknown multi-character names are an
            // error — never silently type the first character.
            let name = raw::keysym_name_for(&key).ok_or_else(|| {
                LghError::InvalidArgument(format!(
                    "empty key name; valid: {}",
                    raw::KEY_VOCABULARY
                ))
            })?;
            let keysym = raw::keysym_for_name(&name).or_else(|| {
                (key.chars().count() == 1)
                    .then(|| raw::keysym_for_char(key.chars().next().unwrap_or('\0')))
                    .flatten()
            }).ok_or_else(|| {
                LghError::InvalidArgument(format!(
                    "unknown key: {key}; valid: {}",
                    raw::KEY_VOCABULARY
                ))
            })?;
            let (keycode, _) = raw::keycode_for_keysym(&mapping, keysym)
                .ok_or_else(|| LghError::InvalidArgument(format!("no keycode for key: {key}")))?;

            let mut codes = Vec::new();
            for m in &modifiers {
                let mname = raw::keysym_name_for(m).ok_or_else(|| {
                    LghError::InvalidArgument(format!("unknown modifier: {m}"))
                })?;
                let mk = raw::keysym_for_name(&mname)
                    .ok_or_else(|| {
                        LghError::InvalidArgument(format!("unknown modifier: {m}"))
                    })?;
                let (mkc, _) = raw::keycode_for_keysym(&mapping, mk).ok_or_else(|| {
                    LghError::InvalidArgument(format!("no keycode for modifier: {m}"))
                })?;
                codes.push(mkc);
            }
            codes.push(keycode);

            for &code in &codes {
                raw::press(&conn, code, true)?;
            }
            for &code in codes.iter().rev() {
                raw::press(&conn, code, false)?;
            }

            raw::deliver(&conn)
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn get_cursor_position(&self) -> Result<(i32, i32), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);
            let reply = conn
                .query_pointer(root)
                .map_err(x11::xerr)?
                .reply()
                .map_err(x11::xerr)?;
            Ok((reply.root_x as i32, reply.root_y as i32))
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }
}

pub struct X11WindowManager {
    display: String,
}

impl X11WindowManager {
    pub fn new(display: &str) -> Result<Self, LghError> {
        let _ = x11::open_connection(display)?;
        Ok(Self {
            display: display.to_string(),
        })
    }
}

#[async_trait]
impl lgh_core::WindowDriver for X11WindowManager {
    async fn focus_window(&self, window_id: u32) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, screen) = x11::open_connection(&display)?;
            let root = x11::root_window(&conn, screen);
            let target = client_window(&conn, window_id);

            let net_active_window = conn
                .intern_atom(false, b"_NET_ACTIVE_WINDOW")
                .map_err(x11::xerr)?
                .reply()
                .map_err(x11::xerr)?
                .atom;

            let event = x11rb::protocol::xproto::ClientMessageEvent::new(
                32,
                target,
                net_active_window,
                [2, x11rb::CURRENT_TIME, 0, 0, 0],
            );
            conn.send_event(
                false,
                root,
                x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT,
                event,
            )
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)?;
            conn.flush().map_err(x11::xerr)?;
            Ok(())
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn set_window_frame(
        &self,
        window_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, _) = x11::open_connection(&display)?;
            let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(x)
                .y(y)
                .width(width)
                .height(height);
            conn.configure_window(window_id, &aux)
                .map_err(x11::xerr)?
                .check()
                .map_err(x11::xerr)?;
            conn.flush().map_err(x11::xerr)?;
            Ok(())
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }

    async fn close_window(&self, window_id: u32) -> Result<(), LghError> {
        let display = self.display.clone();
        task::spawn_blocking(move || {
            let (conn, _) = x11::open_connection(&display)?;

            let wm_protocols = conn
                .intern_atom(false, b"WM_PROTOCOLS")
                .map_err(x11::xerr)?
                .reply()
                .map_err(x11::xerr)?
                .atom;
            let wm_delete_window = conn
                .intern_atom(false, b"WM_DELETE_WINDOW")
                .map_err(x11::xerr)?
                .reply()
                .map_err(x11::xerr)?
                .atom;

            let target = client_window(&conn, window_id);
            let event = x11rb::protocol::xproto::ClientMessageEvent::new(
                32,
                target,
                wm_protocols,
                [wm_delete_window, x11rb::CURRENT_TIME, 0, 0, 0],
            );
            conn.send_event(
                false,
                target,
                x11rb::protocol::xproto::EventMask::NO_EVENT,
                event,
            )
            .map_err(x11::xerr)?
            .check()
            .map_err(x11::xerr)?;
            conn.flush().map_err(x11::xerr)?;
            Ok(())
        })
        .await
        .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::raw;
    use lgh_core::x11;

    #[test]
    fn keysym_lookup() {
        assert!(raw::keysym_for_name("Return").is_some());
        assert!(raw::keysym_for_name("Shift_L").is_some());
        assert!(raw::keysym_for_char('a').is_some());
    }

    #[test]
    fn key_vocabulary_aliases() {
        // case-insensitive aliases resolve to canonical X keysym names
        assert_eq!(raw::keysym_name_for("ENTER").as_deref(), Some("Return"));
        assert_eq!(raw::keysym_name_for("return").as_deref(), Some("Return"));
        assert_eq!(raw::keysym_name_for("pagedown").as_deref(), Some("Next"));
        assert_eq!(raw::keysym_name_for("PGDN").as_deref(), Some("Next"));
        assert_eq!(raw::keysym_name_for("f12").as_deref(), Some("F12"));
        assert_eq!(raw::keysym_name_for("ctrl").as_deref(), Some("Control_L"));
        assert_eq!(raw::keysym_name_for("escape").as_deref(), Some("Escape"));
    }

    #[test]
    fn pagedown_resolves_to_page_down_not_letter_p() {
        // The old first-character fallback turned "pagedown" into "p".
        let name = raw::keysym_name_for("pagedown").unwrap();
        let keysym = raw::keysym_for_name(&name).unwrap();
        assert_eq!(keysym, 0xff56, "Next keysym (XK_Page_Down)");
        assert_ne!(keysym, 'p' as u32);
    }

    #[test]
    fn unknown_key_names_stay_unknown() {
        // no silent first-character fallback for multi-char names
        let name = raw::keysym_name_for("pagedow").unwrap(); // not in the table
        assert!(raw::keysym_for_name(&name).is_none());
    }

    #[test]
    fn keycode_for_keysym_finds_slot_zero() {
        let (conn, _) = x11::open_connection(":0").expect("need local display");
        let mapping = raw::get_mapping(&conn).expect("keyboard mapping");
        let keysym = raw::keysym_for_name("a").unwrap();
        let (code, shifted) = raw::keycode_for_keysym(&mapping, keysym).unwrap();
        assert!(code > 0);
        assert!(!shifted);
    }

    #[test]
    fn keycode_for_keysym_finds_shifted_slot() {
        let (conn, _) = x11::open_connection(":0").expect("need local display");
        let mapping = raw::get_mapping(&conn).expect("keyboard mapping");
        let keysym = raw::keysym_for_name("A").unwrap();
        let (code, shifted) = raw::keycode_for_keysym(&mapping, keysym).unwrap();
        assert!(code > 0);
        assert!(shifted);
    }
}
