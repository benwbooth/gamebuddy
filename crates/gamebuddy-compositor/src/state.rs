use std::{
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use smithay::{
    backend::{renderer::gles::GlesRenderer, winit::WinitGraphicsBackend},
    desktop::{Space, Window},
    input::{Seat, SeatState},
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
        selection::data_device::DataDeviceState,
        shm::ShmState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent},
};
use tracing::{error, info, warn};

use crate::Config;

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
    pub dmabuf: Option<(DmabufState, DmabufGlobal)>,

    pub xwm: Option<X11Wm>,
    pub xdisplay: Option<u32>,
    pub backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub output: Option<Output>,
    pub needs_redraw: bool,
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
            dmabuf: None,
            xwm: None,
            xdisplay: None,
            backend: None,
            output: None,
            needs_redraw: true,
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
        println!("GAMEBUDDY_COMPOSITOR_DISPLAY={display}");
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

    pub fn x11_under(&self, pos: Point<f64, Logical>) -> Option<(X11Surface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .x11_surface()
                    .cloned()
                    .map(|surface| (surface, location.to_f64()))
            })
    }

    pub fn after_dispatch(&mut self) {
        self.space.refresh();
        let _ = self.display_handle.flush_clients();
    }

    pub fn frame_time(&self) -> Duration {
        self.start.elapsed()
    }
}
