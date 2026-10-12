//! The window on the host Wayland desktop. Winit cannot deliver touchpad
//! gestures, relative motion or pointer locks, which GameBuddy needs to pass
//! through to games and the overlay, so on Wayland hosts the compositor is a
//! plain Wayland client built on smithay-client-toolkit.
//!
//! Host events are queued as [`HostEvent`]s and applied by [`crate::State`].

use std::{ffi::c_void, num::NonZeroU32};

use anyhow::{Context, Result};
use smithay::backend::{
    egl::{
        EGLContext, EGLDisplay, EGLSurface,
        context::{GlAttributes, PixelFormatRequirements},
        ffi,
        native::{EGLNativeDisplay, EGLPlatform},
    },
    renderer::gles::GlesRenderer,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    dispatch2::Dispatch2,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keymap, Keysym, Modifiers, RawModifiers},
        pointer::{
            AxisScroll, CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
            ThemedPointer,
        },
        pointer_constraints::{PointerConstraintsHandler, PointerConstraintsState},
        relative_pointer::{RelativeMotionEvent, RelativePointerHandler, RelativePointerState},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    globals::{GlobalList, registry_queue_init},
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    pointer_constraints::zv1::client::{
        zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
    },
    pointer_gestures::zv1::client::{
        zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
        zwp_pointer_gestures_v1,
    },
    relative_pointer::zv1::client::zwp_relative_pointer_v1,
    viewporter::client::{wp_viewport, wp_viewporter},
};

#[derive(Debug)]
pub enum HostEvent {
    /// New physical size of the window contents.
    Resized {
        width: i32,
        height: i32,
    },
    Close,
    Keymap(String),
    Key {
        code: u32,
        pressed: bool,
        time: u32,
    },
    KeyboardLeave,
    /// Position in physical pixels.
    Motion {
        x: f64,
        y: f64,
        time: u32,
    },
    Button {
        button: u32,
        pressed: bool,
        time: u32,
    },
    Axis {
        horizontal: AxisScroll,
        vertical: AxisScroll,
        source: Option<wl_pointer::AxisSource>,
        time: u32,
    },
    Relative {
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
        utime: u64,
    },
    PinchBegin {
        time: u32,
        fingers: u32,
    },
    PinchUpdate {
        time: u32,
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
    },
    PinchEnd {
        time: u32,
        cancelled: bool,
    },
    SwipeBegin {
        time: u32,
        fingers: u32,
    },
    SwipeUpdate {
        time: u32,
        dx: f64,
        dy: f64,
    },
    SwipeEnd {
        time: u32,
        cancelled: bool,
    },
    HoldBegin {
        time: u32,
        fingers: u32,
    },
    HoldEnd {
        time: u32,
        cancelled: bool,
    },
    /// The host is ready for another frame.
    Frame,
}

pub struct Host {
    pub conn: Connection,
    pub qh: QueueHandle<Host>,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    relative_state: RelativePointerState,
    constraints_state: PointerConstraintsState,
    gestures: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    viewport: Option<wp_viewport::WpViewport>,
    _fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    pub window: Window,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    relative: Option<zwp_relative_pointer_v1::ZwpRelativePointerV1>,
    pinch: Option<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1>,
    swipe: Option<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1>,
    hold: Option<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1>,
    lock: Option<zwp_locked_pointer_v1::ZwpLockedPointerV1>,
    /// Logical window size and scale (1.0 = 120/120).
    logical: (i32, i32),
    scale: f64,
    configured: bool,
    cursor: Option<CursorIcon>,
    pub events: Vec<HostEvent>,
}

/// libwayland's display pointer, for EGL.
struct HostDisplay(*mut c_void);
// Safety: only used to create the EGL display on this thread.
unsafe impl Send for HostDisplay {}

impl EGLNativeDisplay for HostDisplay {
    fn supported_platforms(&self) -> Vec<EGLPlatform<'_>> {
        vec![
            EGLPlatform::new(
                ffi::egl::PLATFORM_WAYLAND_KHR,
                "PLATFORM_WAYLAND_KHR",
                self.0,
                vec![ffi::egl::NONE as ffi::EGLint],
                &["EGL_KHR_platform_wayland"],
            ),
            EGLPlatform::new(
                ffi::egl::PLATFORM_WAYLAND_EXT,
                "PLATFORM_WAYLAND_EXT",
                self.0,
                vec![ffi::egl::NONE as ffi::EGLint],
                &["EGL_EXT_platform_wayland"],
            ),
        ]
    }
}

pub struct HostRenderer {
    pub renderer: GlesRenderer,
    pub surface: EGLSurface,
    _display: EGLDisplay,
}

impl Host {
    pub fn connect(fullscreen: bool, size: (i32, i32)) -> Result<(Self, EventQueue<Host>)> {
        let conn =
            Connection::connect_to_env().context("Connecting to the host Wayland desktop")?;
        let (globals, queue) =
            registry_queue_init::<Host>(&conn).context("Reading host globals")?;
        let qh = queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).context("Host has no wl_compositor")?;
        let xdg = XdgShell::bind(&globals, &qh).context("Host has no xdg-shell")?;
        let shm = Shm::bind(&globals, &qh).context("Host has no wl_shm")?;
        let surface = compositor.create_surface(&qh);
        let viewport = bind_optional::<wp_viewporter::WpViewporter>(&globals, &qh, 1)
            .map(|viewporter| viewporter.get_viewport(&surface, &qh, Protocol));
        let fractional =
            bind_optional::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>(
                &globals, &qh, 1,
            )
            .filter(|_| viewport.is_some())
            .map(|manager| manager.get_fractional_scale(&surface, &qh, Protocol));
        let window = xdg.create_window(surface, WindowDecorations::ServerDefault, &qh);
        window.set_title("GameBuddy");
        window.set_app_id("org.gamebuddy.GameBuddy");
        if fullscreen {
            window.set_fullscreen(None);
        }
        window.commit();
        let host = Self {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            relative_state: RelativePointerState::bind(&globals, &qh),
            constraints_state: PointerConstraintsState::bind(&globals, &qh),
            gestures: bind_optional(&globals, &qh, 3),
            viewport,
            _fractional: fractional,
            compositor,
            shm,
            window,
            keyboard: None,
            pointer: None,
            relative: None,
            pinch: None,
            swipe: None,
            hold: None,
            lock: None,
            logical: size,
            scale: 1.0,
            configured: false,
            cursor: None,
            events: Vec::new(),
            qh,
            conn,
        };
        Ok((host, queue))
    }

    /// Block until the host has configured the window, so its size is known.
    pub fn wait_configured(&mut self, queue: &mut EventQueue<Host>) -> Result<()> {
        while !self.configured {
            queue
                .blocking_dispatch(self)
                .context("Waiting for the host window")?;
        }
        Ok(())
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn physical_size(&self) -> (i32, i32) {
        (
            (f64::from(self.logical.0) * self.scale).round() as i32,
            (f64::from(self.logical.1) * self.scale).round() as i32,
        )
    }

    fn apply_size(&mut self) {
        let (w, h) = self.physical_size();
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(self.logical.0, self.logical.1);
        } else {
            self.window
                .wl_surface()
                .set_buffer_scale(self.scale.round().max(1.0) as i32);
        }
        self.events.push(HostEvent::Resized {
            width: w,
            height: h,
        });
    }

    pub fn create_renderer(&self) -> Result<HostRenderer> {
        let display_ptr = self.conn.backend().display_ptr() as *mut c_void;
        let display = unsafe { EGLDisplay::new(HostDisplay(display_ptr)) }
            .context("Creating the EGL display")?;
        let attributes = GlAttributes {
            version: (3, 0),
            profile: None,
            debug: false,
            vsync: false,
        };
        let context =
            EGLContext::new_with_config(&display, attributes, PixelFormatRequirements::_8_bit())
                .context("Creating the EGL context")?;
        let (w, h) = self.physical_size();
        let wl_egl = wayland_egl::WlEglSurface::new(self.window.wl_surface().id(), w, h)
            .context("Creating the window's EGL surface")?;
        let surface = unsafe {
            EGLSurface::new(
                &display,
                context
                    .pixel_format()
                    .context("EGL config has no pixel format")?,
                context.config_id(),
                wl_egl,
            )
        }
        .context("Creating the EGL window surface")?;
        let renderer = unsafe { GlesRenderer::new(context) }.context("Creating the renderer")?;
        Ok(HostRenderer {
            renderer,
            surface,
            _display: display,
        })
    }

    /// Ask for a frame callback; the next [`HostEvent::Frame`] follows it.
    pub fn request_frame(&self) {
        let surface = self.window.wl_surface();
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
    }

    /// Show the host's named cursor, or none while we draw one ourselves.
    pub fn set_cursor(&mut self, icon: Option<CursorIcon>) {
        if self.cursor != icon {
            self.cursor = icon;
            self.apply_cursor();
        }
    }

    fn apply_cursor(&self) {
        if let Some(pointer) = &self.pointer {
            let _ = match self.cursor {
                Some(icon) => pointer.set_cursor(&self.conn, icon),
                None => pointer.hide_cursor(),
            };
        }
    }

    /// Lock the host pointer while a game has locked ours, so relative motion
    /// keeps arriving at the window edges.
    pub fn set_pointer_locked(&mut self, locked: bool) {
        if locked == self.lock.is_some() {
            return;
        }
        if let Some(lock) = self.lock.take() {
            lock.destroy();
            return;
        }
        let Some(pointer) = &self.pointer else {
            return;
        };
        self.lock = self
            .constraints_state
            .lock_pointer(
                self.window.wl_surface(),
                pointer.pointer(),
                None,
                zwp_pointer_constraints_v1::Lifetime::Persistent,
                &self.qh,
            )
            .ok();
    }

    fn to_physical(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x * self.scale, y * self.scale)
    }
}

/// User data for the protocols this file handles itself (sctk handles the rest).
pub struct Protocol;

fn bind_optional<I>(globals: &GlobalList, qh: &QueueHandle<Host>, version: u32) -> Option<I>
where
    I: Proxy + 'static,
    Protocol: Dispatch2<I, Host>,
{
    globals.bind(qh, 1..=version, Protocol).ok()
}

impl CompositorHandler for Host {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        // Integer scales only matter without fractional-scale support.
        if self.viewport.is_none() && f64::from(factor) != self.scale {
            self.scale = f64::from(factor);
            self.apply_size();
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.events.push(HostEvent::Frame);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Host {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for Host {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.events.push(HostEvent::Close);
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let width = configure.new_size.0.map(NonZeroU32::get).map(|v| v as i32);
        let height = configure.new_size.1.map(NonZeroU32::get).map(|v| v as i32);
        let size = (
            width.unwrap_or(self.logical.0),
            height.unwrap_or(self.logical.1),
        );
        if !self.configured || size != self.logical {
            self.logical = size;
            self.apply_size();
        }
        if !self.configured {
            self.configured = true;
            self.events.push(HostEvent::Frame);
        }
    }
}

impl SeatHandler for Host {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            let Ok(pointer) = self.seat_state.get_pointer_with_theme::<Host, ()>(
                qh,
                &seat,
                self.shm.wl_shm(),
                surface,
                ThemeSpec::default(),
            ) else {
                return;
            };
            self.relative = self
                .relative_state
                .get_relative_pointer(pointer.pointer(), qh)
                .ok();
            if let Some(gestures) = &self.gestures {
                self.pinch = Some(gestures.get_pinch_gesture(pointer.pointer(), qh, Protocol));
                self.swipe = Some(gestures.get_swipe_gesture(pointer.pointer(), qh, Protocol));
                if gestures.version() >= 3 {
                    self.hold = Some(gestures.get_hold_gesture(pointer.pointer(), qh, Protocol));
                }
            }
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
            self.events.push(HostEvent::KeyboardLeave);
        }
        if capability == Capability::Pointer {
            self.pointer = None;
            self.relative = None;
            self.pinch = None;
            self.swipe = None;
            self.hold = None;
            self.lock = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.events.push(HostEvent::KeyboardLeave);
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.events.push(HostEvent::Key {
            code: event.raw_code,
            pressed: true,
            time: event.time,
        });
    }

    // Xwayland repeats keys itself from the repeat rate we advertise.
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.events.push(HostEvent::Key {
            code: event.raw_code,
            pressed: false,
            time: event.time,
        });
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }

    fn update_keymap(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        keymap: Keymap<'_>,
    ) {
        self.events.push(HostEvent::Keymap(keymap.as_string()));
    }
}

impl PointerHandler for Host {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let (x, y) = self.to_physical(event.position);
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    // The host forgets our cursor between visits.
                    self.apply_cursor();
                    self.events.push(HostEvent::Motion { x, y, time: 0 });
                }
                PointerEventKind::Leave { .. } => {}
                PointerEventKind::Motion { time } => {
                    self.events.push(HostEvent::Motion { x, y, time })
                }
                PointerEventKind::Press { time, button, .. } => {
                    self.events.push(HostEvent::Button {
                        button,
                        pressed: true,
                        time,
                    })
                }
                PointerEventKind::Release { time, button, .. } => {
                    self.events.push(HostEvent::Button {
                        button,
                        pressed: false,
                        time,
                    })
                }
                PointerEventKind::Axis {
                    time,
                    horizontal,
                    vertical,
                    source,
                } => self.events.push(HostEvent::Axis {
                    horizontal,
                    vertical,
                    source,
                    time,
                }),
            }
        }
    }
}

impl RelativePointerHandler for Host {
    fn relative_pointer_motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        _: &wl_pointer::WlPointer,
        event: RelativeMotionEvent,
    ) {
        self.events.push(HostEvent::Relative {
            dx: event.delta.0 * self.scale,
            dy: event.delta.1 * self.scale,
            dx_unaccel: event.delta_unaccel.0 * self.scale,
            dy_unaccel: event.delta_unaccel.1 * self.scale,
            utime: event.utime,
        });
    }
}

impl PointerConstraintsHandler for Host {
    fn confined(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1,
        _: &wl_surface::WlSurface,
        _: &wl_pointer::WlPointer,
    ) {
    }
    fn unconfined(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1,
        _: &wl_surface::WlSurface,
        _: &wl_pointer::WlPointer,
    ) {
    }
    fn locked(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        _: &wl_surface::WlSurface,
        _: &wl_pointer::WlPointer,
    ) {
    }
    fn unlocked(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        _: &wl_surface::WlSurface,
        _: &wl_pointer::WlPointer,
    ) {
    }
}

impl ShmHandler for Host {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Host);
smithay_client_toolkit::delegate_dispatch2!(Host);

impl ProvidesRegistryState for Host {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

macro_rules! ignore_events {
    ($($proxy:ty),* $(,)?) => {$(
        impl Dispatch2<$proxy, Host> for Protocol {
            fn event(&self, _: &mut Host, _: &$proxy, _: <$proxy as Proxy>::Event, _: &Connection, _: &QueueHandle<Host>) {}
        }
    )*};
}
ignore_events!(
    wp_viewporter::WpViewporter,
    wp_viewport::WpViewport,
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
);

impl Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, Host> for Protocol {
    fn event(
        &self,
        host: &mut Host,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let scale = f64::from(scale) / 120.0;
            if scale != host.scale {
                host.scale = scale;
                host.apply_size();
            }
        }
    }
}

impl Dispatch2<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, Host> for Protocol {
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwp_pointer_gesture_pinch_v1::Event;
        host.events.push(match event {
            Event::Begin { time, fingers, .. } => HostEvent::PinchBegin { time, fingers },
            Event::Update {
                time,
                dx,
                dy,
                scale,
                rotation,
            } => HostEvent::PinchUpdate {
                time,
                dx: dx * host.scale,
                dy: dy * host.scale,
                scale,
                rotation,
            },
            Event::End {
                time, cancelled, ..
            } => HostEvent::PinchEnd {
                time,
                cancelled: cancelled != 0,
            },
            _ => return,
        });
    }
}

impl Dispatch2<zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, Host> for Protocol {
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwp_pointer_gesture_swipe_v1::Event;
        host.events.push(match event {
            Event::Begin { time, fingers, .. } => HostEvent::SwipeBegin { time, fingers },
            Event::Update { time, dx, dy } => HostEvent::SwipeUpdate {
                time,
                dx: dx * host.scale,
                dy: dy * host.scale,
            },
            Event::End {
                time, cancelled, ..
            } => HostEvent::SwipeEnd {
                time,
                cancelled: cancelled != 0,
            },
            _ => return,
        });
    }
}

impl Dispatch2<zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, Host> for Protocol {
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        use zwp_pointer_gesture_hold_v1::Event;
        host.events.push(match event {
            Event::Begin { time, fingers, .. } => HostEvent::HoldBegin { time, fingers },
            Event::End {
                time, cancelled, ..
            } => HostEvent::HoldEnd {
                time,
                cancelled: cancelled != 0,
            },
            _ => return,
        });
    }
}
