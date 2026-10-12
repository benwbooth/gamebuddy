//! Composition shared by the host backends: one output covering the window,
//! the space's X11 windows, the client cursor, and screenshots.

use std::sync::Mutex;

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
            gles::{GlesRenderbuffer, GlesRenderer, GlesTarget},
        },
    },
    desktop::{Space, Window, space::render_output},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::{Mode, Output, PhysicalProperties, Subpixel},
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

pub type Damage = Option<Vec<Rectangle<i32, Physical>>>;

/// Window surfaces are y-flipped relative to GL, so every target uses this.
const TRANSFORM: Transform = Transform::Flipped180;

pub fn create_output(state: &mut State, size: Size<i32, Physical>) -> Output {
    let mode = Mode {
        size,
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
    output.change_current_state(Some(mode), Some(TRANSFORM), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.output = Some(output.clone());
    output
}

/// The host window changed size: refit every game window to it.
pub fn resize_output(state: &mut State, size: Size<i32, Physical>) {
    if let Some(output) = &state.output {
        output.change_current_state(
            Some(Mode {
                size,
                refresh: 60_000,
            }),
            None,
            None,
            None,
        );
    }
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

/// Xwayland's glamor renders on the GPU; give it dmabuf so frames are not
/// copied through shared memory.
pub fn create_dmabuf_global(state: &mut State, renderer: &GlesRenderer) {
    let formats = renderer.dmabuf_formats();
    let mut dmabuf_state = DmabufState::new();
    let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
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
}

/// Frame inputs gathered before the renderer is borrowed.
pub struct FramePlan {
    pub pointer: Point<f64, Logical>,
    pub screenshot: bool,
}

pub fn plan_frame(state: &mut State) -> FramePlan {
    if let CursorImageStatus::Surface(surface) = &state.cursor_status
        && !surface.alive()
    {
        state.cursor_status = CursorImageStatus::default_named();
    }
    FramePlan {
        pointer: state.seat.get_pointer().unwrap().current_location(),
        screenshot: std::mem::take(&mut state.screenshot_requested),
    }
}

pub fn cursor_elements(
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

/// Compose into `framebuffer`; returns the damage to present, or `None` if
/// rendering failed.
pub fn draw(
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
    output: &Output,
    space: &Space<Window>,
    cursor: &[WaylandSurfaceRenderElement<GlesRenderer>],
    tracker: &mut OutputDamageTracker,
) -> Option<Damage> {
    render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
        output,
        renderer,
        framebuffer,
        1.0,
        0,
        [space],
        cursor,
        tracker,
        [0.0, 0.0, 0.0, 1.0],
    )
    .ok()
    .map(|result| result.damage.cloned())
}

/// Compose the current frame (game, overlay and cursor) offscreen and save
/// it where Gamescope writes its debug screenshot, `/tmp/gamescope.png`,
/// which the overlay's compositor probes read.
pub fn screenshot(
    renderer: &mut GlesRenderer,
    output: &Output,
    space: &Space<Window>,
    cursor: &[WaylandSurfaceRenderElement<GlesRenderer>],
    size: Size<i32, Physical>,
) {
    match capture(renderer, output, space, cursor, size) {
        Some((pixels, w, h)) => save_screenshot(pixels, w, h),
        None => warn!("Could not capture a screenshot"),
    }
}

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
    let mut tracker = OutputDamageTracker::new(size, 1.0, TRANSFORM);
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

/// Tell clients their frame was shown so they draw the next one.
pub fn send_frames(state: &State) {
    let Some(output) = &state.output else {
        return;
    };
    let time = state.frame_time();
    state.space.elements().for_each(|window| {
        window.send_frame(output, time, Some(std::time::Duration::ZERO), |_, _| {
            Some(output.clone())
        })
    });
}
