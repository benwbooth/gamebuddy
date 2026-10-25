//! Gamescope's supported Steam overlay properties on its private Xwayland.
//! No desktop-global hooks and no injection into the game's graphics process.
use anyhow::{Context, Result, ensure};
use image::{ImageEncoder, Rgb, RgbImage};
use std::path::Path;
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        composite::ConnectionExt as _,
        res::{ClientIdMask, ClientIdSpec, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

pub fn atom(conn: &RustConnection, name: &str) -> Result<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}
pub fn cardinal(conn: &RustConnection, window: Window, property: Atom) -> Result<u32> {
    Ok(conn
        .get_property(false, window, property, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut v| v.next())
        .unwrap_or(0))
}

/// Top-level client windows. Gamescope maps clients directly on the root,
/// while GameBuddy's own compositor (like most window managers) reparents
/// each one into a frame, so look one level inside root children too.
pub fn client_windows(conn: &RustConnection, root: Window) -> Result<Vec<Window>> {
    let mut windows = Vec::new();
    for child in conn.query_tree(root)?.reply()?.children {
        windows.push(child);
        if let Ok(tree) = conn.query_tree(child)?.reply() {
            windows.extend(tree.children);
        }
    }
    Ok(windows)
}

pub struct Overlay {
    conn: RustConnection,
    root: Window,
    window: Window,
    focus: Atom,
    cursor_visible: Atom,
    mode: u32,
    tab: Keycode,
    escape: Keycode,
    control: Option<crate::control::Endpoint>,
    /// Where the pointer was and when it last moved: a player moving the
    /// mouse means to use it even while the game hides its cursor.
    pointer: [i32; 2],
    moved_at: std::time::Instant,
    /// Last time a pointer over a widget was recorded as not reaching it.
    missed_at: Option<std::time::Instant>,
    /// Input-routing events for the flight recorder, drained by the app.
    events: Vec<(&'static str, serde_json::Value)>,
}
impl Overlay {
    pub fn attach(window: Window) -> Result<Self> {
        ensure!(
            std::env::var_os("GAMEBUDDY_GAMESCOPE").is_some(),
            "Overlay registration is restricted to a GameBuddy Gamescope session"
        );
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let overlay = atom(&conn, "STEAM_OVERLAY")?;
        let focus = atom(&conn, "STEAM_INPUT_FOCUS")?;
        let cursor_visible = atom(&conn, "GAMESCOPE_CURSOR_VISIBLE_FEEDBACK")?;
        conn.change_property32(PropMode::REPLACE, window, overlay, AtomEnum::CARDINAL, &[1])?
            .check()?;
        conn.change_property32(PropMode::REPLACE, window, focus, AtomEnum::CARDINAL, &[0])?
            .check()?;
        // Tab's physical keycode is resolved from this X server, not hardcoded.
        let setup = conn.setup();
        let mapping = conn
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
            .reply()?;
        let tab = mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .position(|symbols| symbols.contains(&0xff09))
            .context("No Tab key in Xwayland keymap")? as u8
            + setup.min_keycode;
        let escape = mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .position(|symbols| symbols.contains(&0xff1b))
            .context("No Escape key in Xwayland keymap")? as u8
            + setup.min_keycode;
        for modifiers in [ModMask::SHIFT, ModMask::SHIFT | ModMask::CONTROL] {
            for extra in [
                ModMask::default(),
                ModMask::LOCK,
                ModMask::M2,
                ModMask::LOCK | ModMask::M2,
            ] {
                conn.grab_key(
                    false,
                    root,
                    modifiers | extra,
                    tab,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?
                .check()
                .context("Shift+Tab is already grabbed in this Gamescope session")?;
            }
        }
        conn.flush()?;
        eprintln!(
            "GAMEBUDDY_OVERLAY_REGISTERED window={window} display={}",
            std::env::var("DISPLAY")?
        );
        Ok(Self {
            conn,
            root,
            window,
            focus,
            cursor_visible,
            mode: 0,
            tab,
            escape,
            control: std::env::var_os("GAMEBUDDY_CONTROL_SOCKET")
                .map(|path| crate::control::Endpoint::bind(path.into()))
                .transpose()?,
            pointer: [0, 0],
            moved_at: std::time::Instant::now() - std::time::Duration::from_secs(60),
            missed_at: None,
            events: vec![],
        })
    }

    /// 0 routes everything to the game, 1 routes to GameBuddy, 2 routes only
    /// the pointer to GameBuddy. Keep pointer focus during an in-progress drag.
    pub fn poll(&mut self, shown: bool, interactive: bool, regions: &[[i32; 4]]) -> Result<i32> {
        let mut action = 0;
        while let Some(event) = self.conn.poll_for_event()? {
            if let Event::KeyPress(event) = event {
                if event.detail == self.tab {
                    action = if event.state.contains(KeyButMask::CONTROL) {
                        2
                    } else {
                        1
                    };
                } else if event.detail == self.escape {
                    action = 6;
                }
            }
        }
        if let Some(endpoint) = &self.control
            && let Some(command) = endpoint.poll()?
        {
            action = command as i32;
        }
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        let buttons = KeyButMask::BUTTON1 | KeyButMask::BUTTON2 | KeyButMask::BUTTON3;
        let dragging = self.mode != 0 && pointer.mask.intersects(buttons);
        let at = [i32::from(pointer.root_x), i32::from(pointer.root_y)];
        if at != self.pointer {
            self.pointer = at;
            self.moved_at = std::time::Instant::now();
        }
        // Emulators hide the cursor, which used to make widgets unreachable;
        // recent mouse movement counts as a visible cursor.
        let visible = cardinal(&self.conn, self.root, self.cursor_visible)? != 0
            || self.moved_at.elapsed() < std::time::Duration::from_secs(3);
        let mode = if !shown {
            0
        } else {
            focus_mode(
                interactive,
                visible,
                dragging,
                [i32::from(pointer.root_x), i32::from(pointer.root_y)],
                regions,
            )
        };
        // Flight recorder: input routing is invisible otherwise. A pointer
        // resting over a widget that still goes to the game is the failure
        // behind widgets that cannot be dragged.
        let over_widget = regions.iter().skip(1).any(|r| {
            r[2] > 0 && r[3] > 0 && at[0] >= r[0] && at[1] >= r[1] && at[0] < r[0] + r[2] && at[1] < r[1] + r[3]
        });
        if shown
            && mode == 0
            && over_widget
            && self.missed_at.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5))
        {
            self.missed_at = Some(std::time::Instant::now());
            self.events.push((
                "input_missed",
                serde_json::json!({"pointer": at, "cursor_visible": visible, "interactive": interactive, "regions": regions.len()}),
            ));
        }
        if mode != self.mode {
            self.events.push((
                "input_focus",
                serde_json::json!({"mode": mode, "pointer": at, "cursor_visible": visible, "dragging": dragging, "regions": regions.len()}),
            ));
            // WebEngine can consume Escape. Grab it only while interacting,
            // so it always releases the overlay and remains native to the game
            // in passive/hidden mode.
            if mode == 1 {
                self.conn
                    .grab_key(
                        false,
                        self.root,
                        ModMask::ANY,
                        self.escape,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .check()?;
            } else if self.mode == 1 {
                self.conn
                    .ungrab_key(self.escape, self.root, ModMask::ANY)?
                    .check()?;
            }
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    self.window,
                    self.focus,
                    AtomEnum::CARDINAL,
                    &[mode],
                )?
                .check()?;
            self.conn.flush()?;
            self.mode = mode;
        }
        Ok(action)
    }
    /// Input-routing events since the last call, for the flight recorder.
    pub fn take_events(&mut self) -> Vec<(&'static str, serde_json::Value)> {
        std::mem::take(&mut self.events)
    }
    pub fn mode(&self) -> u32 {
        self.mode
    }
}
impl Drop for Overlay {
    fn drop(&mut self) {
        // Release focus even when a UI window is hidden before being destroyed.
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.window,
            self.focus,
            AtomEnum::CARDINAL,
            &[0],
        );
        let _ = self.conn.flush();
    }
}
fn focus_mode(
    interactive: bool,
    cursor_visible: bool,
    dragging: bool,
    p: [i32; 2],
    regions: &[[i32; 4]],
) -> u32 {
    if interactive {
        1
    } else if dragging
        || (cursor_visible
            && regions.iter().any(|r| {
                r[2] > 0
                    && r[3] > 0
                    && p[0] >= r[0]
                    && p[1] >= r[1]
                    && p[0] < r[0].saturating_add(r[2])
                    && p[1] < r[1].saturating_add(r[3])
            }))
    {
        2
    } else {
        0
    }
}

fn belongs_to(mut pid: u32, parent: u32) -> bool {
    for _ in 0..32 {
        if pid == parent {
            return true;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
            return false;
        };
        let Some(next) = rest.split_whitespace().nth(1).and_then(|s| s.parse().ok()) else {
            return false;
        };
        if next <= 1 || next == pid {
            return false;
        }
        pid = next;
    }
    false
}

/// Read the game's redirected pixmap, never the root/composited display.
/// Restrict selection to the launched process and its descendants, excluding
/// Steam/external overlays even if a launcher happens to share their PID.
pub fn capture_game(pid: u32, output: &Path) -> Result<Window> {
    ensure!(pid > 1, "A launched game PID is required for capture");
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let pid_atom = atom(&conn, "_NET_WM_PID")?;
    let overlay_atom = atom(&conn, "STEAM_OVERLAY")?;
    let external_atom = atom(&conn, "GAMESCOPE_EXTERNAL_OVERLAY")?;
    let mut best = None;
    for window in client_windows(&conn, root)? {
        // XRes reports the host PID even for Flatpak's PID namespace. The
        // client-written _NET_WM_PID often contains a container-local PID.
        let window_pid = conn
            .res_query_client_ids(&[ClientIdSpec {
                client: window,
                mask: ClientIdMask::LOCAL_CLIENT_PID,
            }])
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| {
                reply
                    .ids
                    .into_iter()
                    .find_map(|id| id.value.first().copied())
            })
            .unwrap_or_else(|| cardinal(&conn, window, pid_atom).unwrap_or(0));
        if window_pid == 0
            || !belongs_to(window_pid, pid)
            || cardinal(&conn, window, overlay_atom).unwrap_or(0) != 0
            || cardinal(&conn, window, external_atom).unwrap_or(0) != 0
        {
            continue;
        }
        let Ok(attr) = conn.get_window_attributes(window)?.reply() else {
            continue;
        };
        if attr.map_state != MapState::VIEWABLE {
            continue;
        }
        let Ok(geo) = conn.get_geometry(window)?.reply() else {
            continue;
        };
        let area = u32::from(geo.width) * u32::from(geo.height);
        if best.as_ref().is_none_or(|(_, _, _, max)| area > *max) {
            best = Some((window, geo, attr.visual, area));
        }
    }
    let (window, geo, visual, _) =
        best.context("Waiting for a mapped Xwayland surface belonging to the game")?;
    conn.composite_query_version(0, 4)?.reply()?;
    let pixmap = conn.generate_id()?;
    // Gamescope redirects the client window itself. A reparenting window
    // manager redirects the frame instead, so read the client's area from the
    // frame's pixmap when the client cannot be named directly.
    let (x, y) = if conn
        .composite_name_window_pixmap(window, pixmap)?
        .check()
        .is_ok()
    {
        (0, 0)
    } else {
        let parent = conn.query_tree(window)?.reply()?.parent;
        ensure!(
            parent != root,
            "Game surface is not redirected by the compositor"
        );
        conn.composite_name_window_pixmap(parent, pixmap)?
            .check()
            .context("Game surface is not redirected by the compositor")?;
        (geo.x, geo.y)
    };
    let pixels = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            x,
            y,
            geo.width,
            geo.height,
            u32::MAX,
        )?
        .reply();
    conn.free_pixmap(pixmap)?.check()?;
    let pixels = pixels?;
    let setup = conn.setup();
    let format = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == pixels.depth)
        .context("Unknown pixel format")?;
    ensure!(
        format.bits_per_pixel == 32,
        "Unsupported game surface: {} bits per pixel",
        format.bits_per_pixel
    );
    let visual = setup.roots[screen]
        .allowed_depths
        .iter()
        .flat_map(|d| &d.visuals)
        .find(|v| v.visual_id == visual)
        .context("Unknown visual")?;
    let stride = (usize::from(geo.width) * usize::from(format.bits_per_pixel))
        .div_ceil(usize::from(format.scanline_pad))
        * usize::from(format.scanline_pad)
        / 8;
    ensure!(
        pixels.data.len() >= stride * usize::from(geo.height),
        "Truncated X image"
    );
    // Every consumer works at or below 1280 px wide; an ultrawide 5120 px
    // surface converted and PNG-compressed at full size cost most of a second
    // per frame. Keep every Nth pixel up to 2560 px wide.
    let step = u32::from(geo.width).div_ceil(CAPTURE_MAX_WIDTH).max(1);
    let mut image = RgbImage::new(
        u32::from(geo.width).div_ceil(step),
        u32::from(geo.height).div_ceil(step),
    );
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (x, y) = (x * step, y * step);
        let offset = y as usize * stride + x as usize * 4;
        let bytes = pixels.data[offset..offset + 4].try_into()?;
        let value = if setup.image_byte_order == ImageOrder::LSB_FIRST {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        };
        *pixel = Rgb([
            channel(value, visual.red_mask),
            channel(value, visual.green_mask),
            channel(value, visual.blue_mask),
        ]);
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(output)?);
    image::codecs::png::PngEncoder::new_with_quality(
        file,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(window)
}
const CAPTURE_MAX_WIDTH: u32 = 2560;
fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    (((pixel & mask) >> shift) * 255 / (mask >> shift)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_does_not_trap_game_input() {
        let rect = &[[30, 40, 80, 80], [600, 80, 350, 260]];
        assert_eq!(focus_mode(false, true, false, [1, 1], rect), 0);
        assert_eq!(focus_mode(false, true, false, [35, 50], rect), 2);
        assert_eq!(focus_mode(false, false, false, [35, 50], rect), 0);
        assert_eq!(focus_mode(false, true, true, [500, 500], rect), 2);
        assert_eq!(focus_mode(true, false, false, [1, 1], rect), 1);
        assert_eq!(focus_mode(false, true, false, [650, 100], rect), 2);
        assert_eq!(focus_mode(false, true, false, [950, 100], rect), 0);
        assert_eq!(focus_mode(false, true, false, [650, 100], &[]), 0);
    }
    #[test]
    fn decodes_8_and_10_bit_channels() {
        assert_eq!(channel(0x0080ff00, 0xff0000), 128);
        assert_eq!(channel(0x3ff00000, 0x3ff00000), 255);
        assert_eq!(channel(0x20000000, 0x3ff00000), 127);
    }
}
