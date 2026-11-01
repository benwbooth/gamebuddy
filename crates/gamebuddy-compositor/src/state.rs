use std::{
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use smithay::{
    backend::{renderer::gles::GlesRenderer, winit::WinitGraphicsBackend},
    desktop::{Space, Window},
    input::{Seat, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        dmabuf::{DmabufGlobal, DmabufState},
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        relative_pointer::RelativePointerManagerState,
        selection::data_device::DataDeviceState,
        shm::ShmState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent},
};
use tracing::{error, info, warn};

use crate::{Config, host_backend::HostBackend, xprops::Props};

pub enum Backend {
    Winit(Box<WinitGraphicsBackend<GlesRenderer>>),
    Host(Box<HostBackend>),
}

impl Backend {
    pub fn renderer(&mut self) -> &mut GlesRenderer {
        match self {
            Backend::Winit(backend) => backend.renderer(),
            Backend::Host(backend) => backend.renderer(),
        }
    }
}

pub struct State {
    pub start: Instant,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub signal: LoopSignal,
    pub config: Config,

    pub space: Space<Window>,
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub seat: Seat<State>,
    pub data_device_state: DataDeviceState,
    pub xwayland_shell_state: XWaylandShellState,
    pub pointer_gestures_state: PointerGesturesState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub dmabuf: Option<(DmabufState, DmabufGlobal)>,

    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,
    pub backend: Option<Backend>,
    pub output: Option<Output>,
    pub needs_redraw: bool,
    pub props: Option<Props>,
    pub cursor_status: CursorImageStatus,
    pub screenshot_requested: bool,
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl State {
    pub fn new(
        event_loop: &mut EventLoop<'static, State>,
        display: Display<State>,
        config: Config,
    ) -> Result<Self> {
        let display_handle = display.handle();
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&display_handle);
        // Xwayland turns these into XInput gestures, raw motion and grabs.
        let pointer_gestures_state = PointerGesturesState::new::<Self>(&display_handle);
        let relative_pointer_state = RelativePointerManagerState::new::<Self>(&display_handle);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&display_handle);
        let mut seat = seat_state.new_wl_seat(&display_handle, "gamebuddy");
        seat.add_keyboard(Default::default(), 200, 25)
            .context("Creating the seat keyboard")?;
        seat.add_pointer();

        // Xwayland is our only client and connects through the socket pair it
        // is spawned with, so no public WAYLAND_DISPLAY socket is opened.
        event_loop
            .handle()
            .insert_source(
                smithay::reexports::calloop::generic::Generic::new(
                    display,
                    smithay::reexports::calloop::Interest::READ,
                    smithay::reexports::calloop::Mode::Level,
                ),
                |_, display, state| {
                    // Safety: the display is never dropped while the loop runs.
                    unsafe { display.get_mut().dispatch_clients(state).unwrap() };
                    Ok(smithay::reexports::calloop::PostAction::Continue)
                },
            )
            .map_err(|e| anyhow::anyhow!("Watching the Wayland display: {e}"))?;

        Ok(Self {
            start: Instant::now(),
            display_handle,
            loop_handle: event_loop.handle(),
            signal: event_loop.get_signal(),
            config,
            space: Space::default(),
            compositor_state,
            shm_state,
            output_manager_state,
            seat_state,
            seat,
            data_device_state,
            xwayland_shell_state,
            pointer_gestures_state,
            relative_pointer_state,
            pointer_constraints_state,
            dmabuf: None,
            xwm: None,
            xdisplay: None,
            backend: None,
            output: None,
            needs_redraw: true,
            props: None,
            cursor_status: CursorImageStatus::default_named(),
            screenshot_requested: false,
        })
    }

    pub fn start_xwayland(&mut self) -> Result<()> {
        let (xwayland, client) = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::inherit(),
            |_| (),
        )
        .context("Starting Xwayland")?;
        self.loop_handle
            .insert_source(xwayland, move |event, _, state| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let wm = match X11Wm::start_wm(
                        state.loop_handle.clone(),
                        x11_socket,
                        client.clone(),
                    ) {
                        Ok(wm) => wm,
                        Err(err) => {
                            error!(?err, "Could not attach the X11 window manager");
                            state.signal.stop();
                            return;
                        }
                    };
                    state.xwm = Some(wm);
                    state.xdisplay = Some(display_number);
                    let name = format!(":{display_number}");
                    match Props::connect(&name, &state.loop_handle) {
                        Ok(props) => state.props = Some(props),
                        Err(err) => warn!(?err, "Overlay properties are unavailable"),
                    }
                    info!(display = %name, "Xwayland ready");
                    state.publish_ready(&name);
                }
                XWaylandEvent::Error => {
                    error!("Xwayland exited during startup");
                    state.signal.stop();
                }
            })
            .map_err(|e| anyhow::anyhow!("Watching Xwayland: {e}"))?;
        Ok(())
    }

    fn publish_ready(&mut self, display: &str) {
        eprintln!("GAMEBUDDY_COMPOSITOR_DISPLAY={display}");
        if let Some(path) = &self.config.ready_file
            && let Err(err) = std::fs::write(path, display)
        {
            warn!(?err, path = %path.display(), "Could not write the ready file");
        }
        if let Err(err) = crate::spawn_command(&self.config.command, display) {
            error!(?err, "Could not start the requested command");
        }
    }

    pub fn output_size(&self) -> (i32, i32) {
        self.output
            .as_ref()
            .and_then(|output| output.current_mode())
            .map(|mode| (mode.size.w, mode.size.h))
            .unwrap_or((1280, 720))
    }

    /// Size every mapped X11 window to the whole output. Milestone 1 treats
    /// each top-level X11 window as fullscreen, as Gamescope does for games.
    pub fn fit_to_output(&mut self, window: &X11Surface) {
        let (w, h) = self.output_size();
        let rect = smithay::utils::Rectangle::new((0, 0).into(), (w, h).into());
        if let Err(err) = window.configure(Some(rect)) {
            warn!(?err, "Could not configure an X11 window");
        }
    }

    pub fn overlay_mode(&self, surface: &X11Surface) -> Option<u32> {
        self.props
            .as_ref()
            .and_then(|props| props.overlays.get(&surface.window_id()).copied())
    }

    fn x11_windows(&self) -> Vec<X11Surface> {
        self.space
            .elements()
            .filter_map(|window| window.x11_surface().cloned())
            .collect()
    }

    /// The window the pointer belongs to. Overlays only receive the pointer
    /// while they ask for input: mode 1 takes it everywhere, mode 2 inside
    /// the overlay's own window. Otherwise it passes through to the game.
    pub fn x11_under(&self, pos: Point<f64, Logical>) -> Option<(X11Surface, Point<f64, Logical>)> {
        let windows = self.x11_windows();
        if let Some(overlay) = windows
            .iter()
            .rev()
            .find(|w| self.overlay_mode(w) == Some(1))
        {
            let loc = self.space.element_location(
                self.space
                    .elements()
                    .find(|e| e.x11_surface() == Some(overlay))?,
            )?;
            return Some((overlay.clone(), loc.to_f64()));
        }
        self.space
            .elements()
            .rev()
            .filter(|element| {
                let Some(surface) = element.x11_surface() else {
                    return false;
                };
                !matches!(self.overlay_mode(surface), Some(0))
            })
            .find_map(|element| {
                let loc = self.space.element_location(element)?;
                let geometry = smithay::desktop::space::SpaceElement::bbox(element);
                let local = pos - loc.to_f64();
                (local.x >= 0.0
                    && local.y >= 0.0
                    && local.x < f64::from(geometry.size.w)
                    && local.y < f64::from(geometry.size.h))
                .then(|| (element.x11_surface().cloned(), loc.to_f64()))
            })
            .and_then(|(surface, loc)| surface.map(|surface| (surface, loc)))
    }

    /// Keep overlays above the game and give the keyboard to whichever side
    /// currently owns input, as Gamescope does with `STEAM_INPUT_FOCUS`.
    pub fn apply_routing(&mut self) {
        let overlays: Vec<_> = self
            .space
            .elements()
            .filter(|e| {
                e.x11_surface()
                    .is_some_and(|s| self.overlay_mode(s).is_some())
            })
            .cloned()
            .collect();
        for overlay in overlays {
            if let (Some(surface), Some(props)) = (overlay.x11_surface(), self.props.as_mut()) {
                let passes = props.overlays.get(&surface.window_id()) == Some(&0);
                props.set_passes_input(surface.window_id(), passes);
                // The frame around the client would catch the click otherwise.
                if let Some(frame) = surface.mapped_window_id() {
                    props.set_passes_input(frame, passes);
                }
                props.flush();
            }
            self.space.raise_element(&overlay, false);
        }
        let windows = self.x11_windows();
        let target = windows
            .iter()
            .rev()
            .find(|w| self.overlay_mode(w) == Some(1))
            .or_else(|| {
                windows
                    .iter()
                    .rev()
                    .find(|w| self.overlay_mode(w).is_none() && !w.is_override_redirect())
            })
            .cloned();
        let keyboard = self.seat.get_keyboard().unwrap();
        if keyboard.current_focus() != target {
            keyboard.set_focus(self, target, smithay::utils::SERIAL_COUNTER.next_serial());
        }
        // Re-evaluate the pointer target in place so a mode change takes
        // effect without waiting for the next motion event.
        let pointer = self.seat.get_pointer().unwrap();
        // Re-entering a surface makes Xwayland move its pointer to the entry
        // point, so enter where X already has it rather than a stale position.
        let location = self
            .props
            .as_ref()
            .and_then(|props| props.pointer())
            .map(Point::from)
            .unwrap_or_else(|| pointer.current_location());
        let under = self.x11_under(location);
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: self.start.elapsed().as_millis() as u32,
            },
        );
        pointer.frame(self);
        self.needs_redraw = true;
    }

    pub fn drain_props(&mut self) {
        let Some(drained) = self.props.as_mut().map(|props| props.drain()) else {
            return;
        };
        if drained.routing {
            self.apply_routing();
        }
        if drained.screenshot {
            self.screenshot_requested = true;
            self.needs_redraw = true;
        }
    }

    pub fn after_dispatch(&mut self) {
        self.render_host();
        self.flush_host();
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }

    pub fn frame_time(&self) -> Duration {
        self.start.elapsed()
    }
}
