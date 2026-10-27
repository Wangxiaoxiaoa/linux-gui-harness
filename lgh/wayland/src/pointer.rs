//! zwlr_virtual_pointer_v1 session — the sandbox's absolute-pointer input
//! channel (the Wayland counterpart of XTest). A drag is one connection
//! holding one virtual pointer across press/move/release, so applications
//! see a real held-button gesture.

use std::time::Instant;

use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
    zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

struct App {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            if interface == "zwlr_virtual_pointer_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
        }
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for App {
    fn event(
        _: &mut Self, _: &ZwlrVirtualPointerManagerV1,
        event: zwlr_virtual_pointer_manager_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event { _ => {} }
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for App {
    fn event(
        _: &mut Self, _: &ZwlrVirtualPointerV1,
        event: zwlr_virtual_pointer_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event { _ => {} }
    }
}

pub struct VPtrSession {
    conn: Connection,
    queue: wayland_client::EventQueue<App>,
    app: App,
    ptr: ZwlrVirtualPointerV1,
    start: Instant,
    width: u32,
    height: u32,
}

impl VPtrSession {
    pub fn open(wayland_display: &str, width: u32, height: u32) -> Result<Self, lgh_core::LghError> {
        let path = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| std::path::PathBuf::from(d).join(wayland_display))
            .unwrap_or_else(|| std::path::PathBuf::from(wayland_display));
        let stream = std::os::unix::net::UnixStream::connect(&path)
            .map_err(|e| lgh_core::LghError::DisplayUnavailable(format!("virtual pointer: cannot connect to {}: {e}", path.display())))?;
        let conn = Connection::from_socket(stream).map_err(|e| lgh_core::LghError::DisplayUnavailable(e.to_string()))?;
        let mut queue = conn.new_event_queue::<App>();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut app = App { manager: None };
        queue.roundtrip(&mut app).map(|_| ()).map_err(|e| lgh_core::LghError::DisplayUnavailable(e.to_string()))?;
        let manager = app
            .manager
            .take()
            .ok_or(lgh_core::LghError::DisplayUnavailable("compositor does not expose zwlr_virtual_pointer_manager_v1".into()))?;
        let ptr = manager.create_virtual_pointer(None, &qh, ());
        Ok(Self {
            conn, queue, app: App { manager: None },
            ptr, start: Instant::now(), width, height,
        })
    }

    fn t(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn sync(&mut self) -> Result<(), lgh_core::LghError> {
        self.queue.roundtrip(&mut self.app).map(|_| ()).map_err(|e| lgh_core::LghError::DisplayUnavailable(e.to_string()))
    }

    pub fn move_abs(&mut self, x: u32, y: u32) -> Result<(), lgh_core::LghError> {
        let t = self.t();
        self.ptr.motion_absolute(
            t,
            x.min(self.width.saturating_sub(1)),
            y.min(self.height.saturating_sub(1)),
            self.width, self.height,
        );
        self.ptr.frame();
        self.sync()
    }

    pub fn button(&mut self, code: u32, down: bool) -> Result<(), lgh_core::LghError> {
        let t = self.t();
        self.ptr.button(
            t, code,
            if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released },
        );
        self.ptr.frame();
        self.sync()
    }

    pub fn scroll(&mut self, dx: f64, dy: f64) -> Result<(), lgh_core::LghError> {
        let t = self.t();
        if dx != 0.0 {
            self.ptr.axis(t, wl_pointer::Axis::HorizontalScroll, dx * 10.0);
        }
        if dy != 0.0 {
            self.ptr.axis(t, wl_pointer::Axis::VerticalScroll, dy * 10.0);
        }
        self.ptr.frame();
        self.sync()
    }

    pub fn close(mut self) -> Result<(), lgh_core::LghError> {
        self.ptr.destroy();
        self.sync()?;
        Ok(())
    }
}
