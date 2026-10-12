//! The window properties GameBuddy's overlay already speaks to Gamescope:
//! `STEAM_OVERLAY` marks an overlay window, `STEAM_INPUT_FOCUS` asks for
//! input (0 game, 1 keyboard and pointer, 2 pointer only), and the root's
//! `GAMESCOPE_CURSOR_VISIBLE_FEEDBACK` reports whether the game shows a cursor.
//! Keeping these names lets the existing overlay run unchanged.

use std::{collections::HashMap, os::fd::AsRawFd};

use anyhow::{Context, Result};
use smithay::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction,
    generic::{FdWrapper, Generic},
};
use tracing::warn;
use x11rb::{
    connection::Connection,
    protocol::{Event, xproto::*},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::state::State;

#[derive(Default)]
pub struct Drained {
    pub routing: bool,
    /// Gamescope's debug screenshot request, used by the overlay's probes.
    pub screenshot: bool,
}

pub struct Props {
    conn: RustConnection,
    root: Window,
    overlay: Atom,
    focus: Atom,
    cursor_visible: Atom,
    cursor_visible_value: Option<u32>,
    screenshot: Atom,
    /// Overlay windows and their requested input focus mode.
    pub overlays: HashMap<Window, u32>,
}

impl Props {
    pub fn connect(display: &str, handle: &LoopHandle<'static, State>) -> Result<Self> {
        let (conn, screen) =
            RustConnection::connect(Some(display)).context("Connecting to our Xwayland")?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &str| -> Result<Atom> {
            Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
        };
        let server_id = atom("GAMESCOPE_XWAYLAND_SERVER_ID")?;
        let props = Self {
            overlay: atom("STEAM_OVERLAY")?,
            focus: atom("STEAM_INPUT_FOCUS")?,
            cursor_visible: atom("GAMESCOPE_CURSOR_VISIBLE_FEEDBACK")?,
            cursor_visible_value: None,
            screenshot: atom("GAMESCOPECTRL_DEBUG_REQUEST_SCREENSHOT")?,
            overlays: HashMap::new(),
            root,
            conn,
        };
        // The overlay's compositor probe only sends test input to a server
        // that identifies itself as a private session, as Gamescope's does.
        props.conn.change_property32(
            PropMode::REPLACE,
            root,
            server_id,
            AtomEnum::CARDINAL,
            &[0],
        )?;
        props.conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        props.conn.flush()?;
        // Safety: the connection lives in `State::props` for as long as this
        // source is registered; both are dropped together at shutdown.
        let fd = unsafe { FdWrapper::new(props.conn.stream().as_raw_fd()) };
        handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                |_, _, state| {
                    state.drain_props();
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| anyhow::anyhow!("Watching overlay properties: {e}"))?;
        Ok(props)
    }

    /// Start following a newly mapped top-level window's properties.
    pub fn watch(&mut self, window: Window) {
        let watched = self
            .conn
            .change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map(|_| ());
        if let Err(err) = watched {
            warn!(?err, window, "Could not watch an X11 window");
        }
        self.refresh(window);
        let _ = self.conn.flush();
    }

    pub fn forget(&mut self, window: Window) {
        self.overlays.remove(&window);
    }

    fn cardinal(&self, window: Window, property: Atom) -> Option<u32> {
        self.conn
            .get_property(false, window, property, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()
            .and_then(|mut values| values.next())
    }

    fn refresh(&mut self, window: Window) {
        if self.cardinal(window, self.overlay).unwrap_or(0) != 0 {
            let mode = self.cardinal(window, self.focus).unwrap_or(0);
            self.overlays.insert(window, mode);
        } else {
            self.overlays.remove(&window);
        }
    }

    /// Process pending property changes.
    pub fn drain(&mut self) -> Drained {
        let mut changed = Drained::default();
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(Event::PropertyNotify(event)))
                    if event.atom == self.overlay || event.atom == self.focus =>
                {
                    let before = self.overlays.get(&event.window).copied();
                    self.refresh(event.window);
                    changed.routing |= before != self.overlays.get(&event.window).copied();
                }
                Ok(Some(Event::PropertyNotify(event)))
                    if event.window == self.root
                        && event.atom == self.screenshot
                        && event.state == Property::NEW_VALUE =>
                {
                    changed.screenshot = true;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(err) => {
                    warn!(?err, "Lost the overlay property connection");
                    break;
                }
            }
        }
        changed
    }

    /// Where Xwayland believes the pointer is. Usually this is our own pointer
    /// position, but XTest input (the overlay's probes) moves it inside X.
    pub fn pointer(&self) -> Option<(f64, f64)> {
        let reply = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((f64::from(reply.root_x), f64::from(reply.root_y)))
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        let value = u32::from(visible);
        if self.cursor_visible_value == Some(value) {
            return;
        }
        self.cursor_visible_value = Some(value);
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.cursor_visible,
            AtomEnum::CARDINAL,
            &[value],
        );
        let _ = self.conn.flush();
    }
}
