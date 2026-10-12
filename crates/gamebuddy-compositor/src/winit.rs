use std::time::Duration;

use anyhow::{Context, Result};
use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{
            ImportDma, damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::space::render_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        winit::{
            dpi::PhysicalSize,
            window::{Fullscreen, Window as WinitWindow},
        },
    },
    utils::Transform,
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufState},
};
use tracing::warn;

use crate::state::State;

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

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };
    let output = Output::new(
        "gamebuddy".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "GameBuddy".into(),
            model: "Nested".into(),
        },
    );
    let _global = output.create_global::<State>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    // Xwayland's glamor renders on the GPU; give it dmabuf so frames are not
    // copied through shared memory.
    let formats = backend.renderer().dmabuf_formats();
    let mut dmabuf_state = DmabufState::new();
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten());
    let global = match render_node.and_then(|node| {
        DmabufFeedbackBuilder::new(node.dev_id(), formats.clone())
            .build()
            .ok()
    }) {
        Some(feedback) => dmabuf_state
            .create_global_with_default_feedback::<State>(&state.display_handle, &feedback),
        None => dmabuf_state.create_global::<State>(&state.display_handle, formats),
    };
    state.dmabuf = Some((dmabuf_state, global));

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    state.output = Some(output.clone());
    state.backend = Some(backend);

    event_loop
        .handle()
        .insert_source(winit_loop, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(
                    Some(Mode {
                        size,
                        refresh: 60_000,
                    }),
                    None,
                    None,
                    None,
                );
                let windows: Vec<_> = state
                    .space
                    .elements()
                    .filter_map(|w| w.x11_surface().cloned())
                    .filter(|s| !s.is_override_redirect())
                    .collect();
                for window in windows {
                    state.fit_to_output(&window);
                }
                state.needs_redraw = true;
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let time = state.frame_time();
                let Some(backend) = state.backend.as_mut() else {
                    return;
                };
                let rendered = backend.bind().ok().and_then(|(renderer, mut framebuffer)| {
                    render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
                        &output,
                        renderer,
                        &mut framebuffer,
                        1.0,
                        0,
                        [&state.space],
                        &[],
                        &mut damage_tracker,
                        [0.0, 0.0, 0.0, 1.0],
                    )
                    .ok()
                });
                if let Some(result) = rendered {
                    let damage = result.damage.cloned();
                    if let Err(err) = backend.submit(damage.as_deref()) {
                        warn!(?err, "Could not present a frame");
                    }
                }
                state.space.elements().for_each(|window| {
                    window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    })
                });
                state.needs_redraw = false;
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.signal.stop(),
            WinitEvent::Focus(_) => {}
        })
        .map_err(|e| anyhow::anyhow!("Watching the compositor window: {e}"))
        .context("Starting the host window")?;
    Ok(())
}
