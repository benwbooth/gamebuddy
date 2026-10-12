//! Fallback host window through winit, used on X11 desktops.

use anyhow::{Context, Result};
use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    input::pointer::CursorImageStatus,
    reexports::{
        calloop::EventLoop,
        winit::{
            dpi::PhysicalSize,
            window::{Fullscreen, Window as WinitWindow},
        },
    },
};
use tracing::warn;

use crate::{
    render,
    state::{Backend, State},
};

pub fn init(event_loop: &mut EventLoop<'static, State>, state: &mut State) -> Result<()> {
    let mut attributes = WinitWindow::default_attributes()
        .with_title("GameBuddy")
        .with_visible(true);
    if state.config.fullscreen {
        attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
    } else {
        let (w, h) = state.config.size.unwrap_or((1280, 720));
        attributes = attributes.with_inner_size(PhysicalSize::new(w as u32, h as u32));
    }
    let (mut backend, winit_loop) = winit::init_from_attributes::<GlesRenderer>(attributes)
        .map_err(|e| anyhow::anyhow!("Opening the compositor window: {e}"))?;

    let output = render::create_output(state, backend.window_size());
    render::create_dmabuf_global(state, backend.renderer());
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    state.backend = Some(Backend::Winit(Box::new(backend)));

    event_loop
        .handle()
        .insert_source(winit_loop, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => render::resize_output(state, size),
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let plan = render::plan_frame(state);
                let Some(Backend::Winit(backend)) = state.backend.as_mut() else {
                    return;
                };
                // X11 clients draw their cursors as surfaces; draw those in the
                // game, and let the host show named cursors or nothing.
                match &state.cursor_status {
                    CursorImageStatus::Hidden | CursorImageStatus::Surface(_) => {
                        backend.window().set_cursor_visible(false)
                    }
                    CursorImageStatus::Named(icon) => {
                        backend.window().set_cursor_visible(true);
                        backend.window().set_cursor(*icon);
                    }
                }
                let size = backend.window_size();
                let cursor =
                    render::cursor_elements(backend.renderer(), &state.cursor_status, plan.pointer);
                if plan.screenshot {
                    render::screenshot(backend.renderer(), &output, &state.space, &cursor, size);
                }
                let damage = backend.bind().ok().and_then(|(renderer, mut framebuffer)| {
                    render::draw(
                        renderer,
                        &mut framebuffer,
                        &output,
                        &state.space,
                        &cursor,
                        &mut damage_tracker,
                    )
                });
                if let Some(damage) = damage
                    && let Err(err) = backend.submit(damage.as_deref())
                {
                    warn!(?err, "Could not present a frame");
                }
                backend.window().request_redraw();
                render::send_frames(state);
                state.needs_redraw = false;
            }
            WinitEvent::CloseRequested => state.signal.stop(),
            WinitEvent::Focus(_) => {}
        })
        .map_err(|e| anyhow::anyhow!("Watching the compositor window: {e}"))
        .context("Starting the host window")?;
    Ok(())
}
