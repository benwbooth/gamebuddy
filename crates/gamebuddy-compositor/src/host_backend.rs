//! Drives the [`Host`] window from the compositor's event loop.

use std::os::fd::AsRawFd;

use anyhow::Result;
use smithay::{
    backend::renderer::{Bind, damage::OutputDamageTracker, gles::GlesRenderer},
    input::pointer::CursorImageStatus,
    reexports::calloop::{
        EventLoop, Interest, Mode, PostAction,
        generic::{FdWrapper, Generic},
    },
    utils::{Physical, Size},
};
use tracing::warn;
use wayland_client::EventQueue;

use crate::{
    host::{Host, HostEvent, HostRenderer},
    render,
    state::{Backend, State},
};

pub struct HostBackend {
    pub host: Host,
    queue: EventQueue<Host>,
    pub gl: HostRenderer,
    tracker: OutputDamageTracker,
    size: Size<i32, Physical>,
    frame_pending: bool,
}

impl HostBackend {
    pub fn renderer(&mut self) -> &mut GlesRenderer {
        &mut self.gl.renderer
    }
}

pub fn init(event_loop: &mut EventLoop<'static, State>, state: &mut State) -> Result<()> {
    let (mut host, mut queue) = Host::connect(
        state.config.fullscreen,
        state.config.size.unwrap_or((1280, 720)),
    )?;
    host.wait_configured(&mut queue)?;
    let gl = host.create_renderer()?;
    let size: Size<i32, Physical> = host.physical_size().into();
    let output = render::create_output(state, size);
    render::create_dmabuf_global(state, &gl.renderer);
    let tracker = OutputDamageTracker::from_output(&output);

    // Safety: the connection lives in `State::backend` for as long as the
    // event loop runs, which is as long as this source stays registered.
    let fd = unsafe { FdWrapper::new(host.conn.backend().poll_fd().as_raw_fd()) };
    event_loop
        .handle()
        .insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, state| {
                state.dispatch_host();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("Watching the host connection: {e}"))?;
    state.backend = Some(Backend::Host(Box::new(HostBackend {
        host,
        queue,
        gl,
        tracker,
        size,
        frame_pending: false,
    })));
    state.dispatch_host();
    Ok(())
}

impl State {
    pub fn dispatch_host(&mut self) {
        let Some(Backend::Host(backend)) = self.backend.as_mut() else {
            return;
        };
        if let Some(guard) = backend.queue.prepare_read()
            && let Err(err) = guard.read()
            && !matches!(err, wayland_client::backend::WaylandError::Io(ref io) if io.kind() == std::io::ErrorKind::WouldBlock)
        {
            warn!(?err, "Lost the host desktop connection");
            self.signal.stop();
            return;
        }
        if let Err(err) = backend.queue.dispatch_pending(&mut backend.host) {
            warn!(?err, "Host desktop protocol error");
            self.signal.stop();
            return;
        }
        let events = std::mem::take(&mut backend.host.events);
        for event in events {
            self.handle_host_event(event);
        }
    }

    fn handle_host_event(&mut self, event: HostEvent) {
        match event {
            HostEvent::Resized { width, height } => {
                if let Some(Backend::Host(backend)) = self.backend.as_mut() {
                    backend.size = (width, height).into();
                    backend.gl.surface.resize(width, height, 0, 0);
                }
                render::resize_output(self, (width, height).into());
            }
            HostEvent::Frame => {
                if let Some(Backend::Host(backend)) = self.backend.as_mut() {
                    backend.frame_pending = false;
                }
                self.needs_redraw = true;
            }
            HostEvent::Close => self.signal.stop(),
            event => self.process_host_input(event),
        }
    }

    /// Render when something changed and the host can take a frame.
    pub fn render_host(&mut self) {
        let ready = matches!(
            self.backend.as_ref(),
            Some(Backend::Host(backend)) if !backend.frame_pending
        );
        if !ready || !self.needs_redraw {
            return;
        }
        let plan = render::plan_frame(self);
        let Some(output) = self.output.clone() else {
            return;
        };
        let Some(Backend::Host(backend)) = self.backend.as_mut() else {
            return;
        };
        backend.host.set_cursor(match &self.cursor_status {
            CursorImageStatus::Named(icon) => Some(*icon),
            _ => None,
        });
        let cursor =
            render::cursor_elements(&mut backend.gl.renderer, &self.cursor_status, plan.pointer);
        if plan.screenshot {
            render::screenshot(
                &mut backend.gl.renderer,
                &output,
                &self.space,
                &cursor,
                backend.size,
            );
        }
        let damage = match backend.gl.renderer.bind(&mut backend.gl.surface) {
            Ok(mut framebuffer) => render::draw(
                &mut backend.gl.renderer,
                &mut framebuffer,
                &output,
                &self.space,
                &cursor,
                &mut backend.tracker,
            ),
            Err(err) => {
                warn!(?err, "Could not bind the host window");
                None
            }
        };
        if let Some(mut damage) = damage {
            backend.host.request_frame();
            backend.frame_pending = true;
            if let Err(err) = backend.gl.surface.swap_buffers(damage.as_deref_mut()) {
                warn!(?err, "Could not present a frame");
            }
        }
        render::send_frames(self);
        self.needs_redraw = false;
    }

    pub fn flush_host(&mut self) {
        if let Some(Backend::Host(backend)) = self.backend.as_mut() {
            let _ = backend.host.conn.flush();
        }
    }
}

/// Whether to use the native Wayland host window rather than winit.
pub fn available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty())
}
