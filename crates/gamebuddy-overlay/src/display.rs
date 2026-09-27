//! Read physical display modes without creating a window or changing the host.
//! Desktop logical dimensions lose pixels under fractional scaling and must not
//! be used as the game render resolution.
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, sync::mpsc, time::Duration};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_output, wl_registry},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}
impl Size {
    pub const WINDOWED: Self = Self {
        width: 1280,
        height: 720,
    };

    pub fn with_overrides(self, width: Option<u32>, height: Option<u32>) -> Self {
        match (width, height) {
            (Some(width), Some(height)) => Self { width, height },
            (Some(width), None) => Self {
                width,
                height: ((u64::from(width) * u64::from(self.height) + u64::from(self.width) / 2)
                    / u64::from(self.width))
                .max(1) as u32,
            },
            (None, Some(height)) => Self {
                width: ((u64::from(height) * u64::from(self.width) + u64::from(self.height) / 2)
                    / u64::from(self.height))
                .max(1) as u32,
                height,
            },
            (None, None) => self,
        }
    }
}

/// Prefer the native Wayland mode over Xwayland's potentially scaled desktop.
/// Bound discovery so a stalled display server cannot stall the launcher.
pub fn native_size() -> Result<Size> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let result = if wayland { wayland_size() } else { x11_size() };
        let _ = send.send(result);
    });
    receive
        .recv_timeout(Duration::from_secs(3))
        .context("Display mode query timed out")?
}

#[derive(Default)]
struct Output {
    x: i32,
    y: i32,
    mode: Option<Size>,
    rotated: bool,
}
impl Output {
    fn size(&self) -> Option<Size> {
        self.mode.map(|mode| {
            if self.rotated {
                Size {
                    width: mode.height,
                    height: mode.width,
                }
            } else {
                mode
            }
        })
    }
}

#[derive(Default)]
struct Outputs(BTreeMap<u32, Output>);
impl Dispatch<wl_registry::WlRegistry, ()> for Outputs {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "wl_output"
        {
            state.0.insert(name, Output::default());
            registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), queue, name);
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for Outputs {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let output = state.0.entry(*id).or_default();
        match event {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                output.x = x;
                output.y = y;
                output.rotated = matches!(
                    transform,
                    WEnum::Value(
                        wl_output::Transform::_90
                            | wl_output::Transform::_270
                            | wl_output::Transform::Flipped90
                            | wl_output::Transform::Flipped270
                    )
                );
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) && width > 0 && height > 0 => {
                output.mode = Some(Size {
                    width: width as u32,
                    height: height as u32,
                });
            }
            _ => {}
        }
    }
}

fn wayland_size() -> Result<Size> {
    let conn = Connection::connect_to_env().context("Connecting to the host Wayland display")?;
    let mut queue = conn.new_event_queue();
    let _registry = conn.display().get_registry(&queue.handle(), ());
    let mut state = Outputs::default();
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    // wl_output has no primary flag. Prefer the desktop origin, then the first
    // active output. Explicit launch dimensions remain available for other outputs.
    state
        .0
        .values()
        .filter(|o| o.size().is_some())
        .min_by_key(|o| (o.x != 0 || o.y != 0, o.x, o.y))
        .and_then(Output::size)
        .context("Wayland did not report an active physical display mode")
}

fn x11_size() -> Result<Size> {
    use x11rb::{connection::Connection as _, protocol::randr::ConnectionExt as _};
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let monitors = conn.randr_get_monitors(root, true)?.reply()?.monitors;
    let monitor = monitors
        .iter()
        .filter(|m| m.width > 0 && m.height > 0)
        .min_by_key(|m| (!m.primary, m.x != 0 || m.y != 0, m.x, m.y))
        .context("X11 did not report an active monitor")?;
    ensure!(
        monitor.width > 0 && monitor.height > 0,
        "Invalid monitor dimensions"
    );
    Ok(Size {
        width: u32::from(monitor.width),
        height: u32::from(monitor.height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_preserve_ultrawide_aspect_and_native_pixels() {
        let native = Size {
            width: 5120,
            height: 2160,
        };
        assert_eq!(native.with_overrides(None, None), native);
        assert_eq!(
            native.with_overrides(Some(2560), None),
            Size {
                width: 2560,
                height: 1080
            }
        );
        assert_eq!(
            native.with_overrides(None, Some(1080)),
            Size {
                width: 2560,
                height: 1080
            }
        );
        assert_eq!(
            native.with_overrides(Some(1920), Some(1080)),
            Size {
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn rotated_output_uses_portrait_pixels() {
        let output = Output {
            mode: Some(Size {
                width: 3840,
                height: 2160,
            }),
            rotated: true,
            ..Default::default()
        };
        assert_eq!(
            output.size(),
            Some(Size {
                width: 2160,
                height: 3840
            })
        );
    }
}
