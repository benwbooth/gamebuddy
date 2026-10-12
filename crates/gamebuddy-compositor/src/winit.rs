use std::{sync::Mutex, time::Duration};

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::Fourcc,
        egl::EGLDevice,
        renderer::{
            Bind, ExportMem, ImportDma, Offscreen, TextureMapping,
            damage::OutputDamageTracker,
            element::{
                Kind,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{GlesRenderbuffer, GlesRenderer},
        },
        winit::{self, WinitEvent},
    },
    desktop::{Space, Window, space::render_output},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        winit::{
            dpi::PhysicalSize,
            window::{Fullscreen, Window as WinitWindow},
        },
    },
    utils::{
        Buffer as BufferCoords, IsAlive, Logical, Physical, Point, Rectangle, Size, Transform,
    },
    wayland::{
        compositor::with_states,
        dmabuf::{DmabufFeedbackBuilder, DmabufState},
    },
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
                if let CursorImageStatus::Surface(surface) = &state.cursor_status
                    && !surface.alive()
                {
                    state.cursor_status = CursorImageStatus::default_named();
                }
                let pointer = state.seat.get_pointer().unwrap().current_location();
                let Some(backend) = state.backend.as_mut() else {
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
                let want_shot = std::mem::take(&mut state.screenshot_requested);
                let size = backend.window_size();
                let cursor = cursor_elements(backend.renderer(), &state.cursor_status, pointer);
                // Screenshots render into their own buffer so reading pixels
                // back never disturbs the window's presentation surface.
                if want_shot {
                    match capture(backend.renderer(), &output, &state.space, &cursor, size) {
                        Some((pixels, w, h)) => save_screenshot(pixels, w, h),
                        None => warn!("Could not capture a screenshot"),
                    }
                }
                let rendered = backend.bind().ok().and_then(|(renderer, mut framebuffer)| {
                    render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
                        &output,
                        renderer,
                        &mut framebuffer,
                        1.0,
                        0,
                        [&state.space],
                        &cursor,
                        &mut damage_tracker,
                        [0.0, 0.0, 0.0, 1.0],
                    )
                    .ok()
                    .map(|result| result.damage.cloned())
                });
                if let Some(damage) = rendered {
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

fn cursor_elements(
    renderer: &mut GlesRenderer,
    status: &CursorImageStatus,
    pointer: Point<f64, Logical>,
) -> Vec<WaylandSurfaceRenderElement<GlesRenderer>> {
    let CursorImageStatus::Surface(surface) = status else {
        return Vec::new();
    };
    let hotspot = with_states(surface, |states| {
        states
            .data_map
            .get::<Mutex<CursorImageAttributes>>()
            .map(|attrs| attrs.lock().unwrap().hotspot)
            .unwrap_or_default()
    });
    render_elements_from_surface_tree(
        renderer,
        surface,
        (pointer - hotspot.to_f64()).to_physical(1.0).to_i32_round(),
        1.0,
        1.0,
        Kind::Cursor,
    )
}

/// Compose the current frame (game, overlay and cursor) offscreen and read
/// it back as RGBA rows from the top of the screen.
fn capture(
    renderer: &mut GlesRenderer,
    output: &Output,
    space: &Space<Window>,
    cursor: &[WaylandSurfaceRenderElement<GlesRenderer>],
    size: Size<i32, Physical>,
) -> Option<(Vec<u8>, u32, u32)> {
    let buffer_size = Size::<i32, BufferCoords>::from((size.w, size.h));
    let mut target: GlesRenderbuffer =
        renderer.create_buffer(Fourcc::Abgr8888, buffer_size).ok()?;
    // Same orientation as the window's EGL surface, which reads back top-first.
    let mut tracker = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
    let mut framebuffer = renderer.bind(&mut target).ok()?;
    render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
        output,
        renderer,
        &mut framebuffer,
        1.0,
        0,
        [space],
        cursor,
        &mut tracker,
        [0.0, 0.0, 0.0, 1.0],
    )
    .ok()?;
    let mapping = renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(buffer_size),
            Fourcc::Abgr8888,
        )
        .ok()?;
    let flipped = mapping.flipped();
    let bytes = renderer.map_texture(&mapping).ok()?;
    let stride = size.w as usize * 4;
    let mut pixels = Vec::with_capacity(bytes.len());
    if flipped {
        for row in bytes.chunks_exact(stride).rev() {
            pixels.extend_from_slice(row);
        }
    } else {
        pixels.extend_from_slice(bytes);
    }
    Some((pixels, size.w as u32, size.h as u32))
}

/// Gamescope writes its debug screenshot to this fixed path; the overlay's
/// compositor probes read it from there.
fn save_screenshot(pixels: Vec<u8>, w: u32, h: u32) {
    let Some(image) = image::RgbaImage::from_raw(w, h, pixels) else {
        return;
    };
    let path = std::path::Path::new("/tmp/gamescope.png");
    let temporary = path.with_extension("png.partial");
    let saved = image
        .save_with_format(&temporary, image::ImageFormat::Png)
        .map_err(anyhow::Error::from)
        .and_then(|()| std::fs::rename(&temporary, path).map_err(anyhow::Error::from));
    if let Err(err) = saved {
        warn!(?err, "Could not save a screenshot");
    }
}
