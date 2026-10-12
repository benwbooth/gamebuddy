use std::os::unix::io::OwnedFd;

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{ImportDma, utils::on_commit_buffer_handler},
    },
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_output, delegate_seat,
    delegate_shm, delegate_xwayland_shell,
    desktop::Window,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::{
        Client, Resource,
        protocol::{wl_buffer::WlBuffer, wl_surface::WlSurface},
    },
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        selection::{
            SelectionHandler, SelectionTarget,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
        },
        shm::{ShmHandler, ShmState},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::state::{ClientState, State};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &state.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
        {
            window.on_commit();
        }
        self.needs_redraw = true;
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for State {
    type KeyboardFocus = X11Surface;
    type PointerFocus = X11Surface;
    type TouchFocus = X11Surface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&X11Surface>) {
        let client = focused
            .and_then(|surface| surface.wl_surface())
            .and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        _ty: SelectionTarget,
        _source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl OutputHandler for State {}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self
            .dmabuf
            .as_mut()
            .expect("dmabuf global is created with the renderer")
            .0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let imported = self
            .backend
            .as_mut()
            .is_some_and(|backend| backend.renderer().import_dmabuf(&dmabuf, None).is_ok());
        if imported {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("the X11 window manager is running")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_mapped(true);
        self.fit_to_output(&window);
        let element = Window::new_x11_window(window.clone());
        self.space.map_element(element, (0, 0), true);
        // The newest top-level window takes the keyboard, as in a game session.
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, Some(window), SERIAL_COUNTER.next_serial());
        self.needs_redraw = true;
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        self.space
            .map_element(Window::new_x11_window(window), location, true);
        self.needs_redraw = true;
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let element = self
            .space
            .elements()
            .find(|e| e.x11_surface() == Some(&window))
            .cloned();
        if let Some(element) = element {
            self.space.unmap_elem(&element);
        }
        if !window.is_override_redirect() {
            let _ = window.set_mapped(false);
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        if keyboard.current_focus().as_ref() == Some(&window) {
            let next = self
                .space
                .elements()
                .filter_map(|e| e.x11_surface().cloned())
                .rfind(|s| !s.is_override_redirect());
            keyboard.set_focus(self, next, SERIAL_COUNTER.next_serial());
        }
        self.needs_redraw = true;
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if window.is_mapped() && !window.is_override_redirect() {
            // Mapped top-level windows stay fullscreen.
            self.fit_to_output(&window);
            return;
        }
        let mut geometry = window.geometry();
        if let Some(w) = w {
            geometry.size.w = w as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h as i32;
        }
        let _ = window.configure(geometry);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let element = self
            .space
            .elements()
            .find(|e| e.x11_surface() == Some(&window))
            .cloned();
        if let Some(element) = element {
            self.space.map_element(element, geometry.loc, false);
            self.needs_redraw = true;
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edges: ResizeEdge,
    ) {
    }
    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_fullscreen(true);
        self.fit_to_output(&window);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _ = window.set_fullscreen(false);
        self.fit_to_output(&window);
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        _selection: SelectionTarget,
        _mime_type: String,
        _fd: OwnedFd,
    ) {
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_dmabuf!(State);
delegate_xwayland_shell!(State);
