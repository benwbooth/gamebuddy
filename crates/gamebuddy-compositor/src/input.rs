use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{FilterResult, Keycode},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            RelativeMotionEvent,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint},
};
use smithay_client_toolkit::{reexports::client::protocol::wl_pointer, seat::pointer::AxisScroll};

use crate::{
    host::HostEvent,
    state::{Backend, State},
};

impl State {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                self.key(event.key_code(), event.state(), event.time_msec())
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let (w, h) = self.output_size();
                let location = event.position_transformed((w, h).into());
                self.pointer_motion(location, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(event.button_code(), event.state(), event.time_msec())
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v120 = event.amount_v120(axis);
                    let amount = event
                        .amount(axis)
                        .unwrap_or_else(|| v120.unwrap_or(0.0) * 15.0 / 120.0);
                    if amount != 0.0 {
                        frame = frame.value(axis, amount);
                        // Keep high-resolution wheel steps intact for clients.
                        if let Some(v120) = v120 {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                self.pointer_axis(frame);
            }
            _ => {}
        }
    }

    pub fn process_host_input(&mut self, event: HostEvent) {
        let pointer = self.seat.get_pointer().unwrap();
        match event {
            HostEvent::Keymap(keymap) => {
                let keyboard = self.seat.get_keyboard().unwrap();
                if let Err(err) = keyboard.set_keymap_from_string(self, keymap) {
                    tracing::warn!(?err, "Could not use the host keymap");
                }
            }
            HostEvent::Key {
                code,
                pressed,
                time,
            } => {
                let state = if pressed {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                };
                // Wayland sends evdev codes; xkb keycodes are offset by 8.
                self.key(Keycode::new(code + 8), state, time);
            }
            HostEvent::KeyboardLeave => {
                // Release anything held so the game never sees a stuck key.
                let keyboard = self.seat.get_keyboard().unwrap();
                let held: Vec<_> = keyboard.pressed_keys().into_iter().collect();
                for code in held {
                    self.key(code, KeyState::Released, self.now());
                }
            }
            HostEvent::Motion { x, y, time } => {
                if !self.pointer_locked() {
                    self.pointer_motion((x, y).into(), time);
                }
            }
            HostEvent::Button {
                button,
                pressed,
                time,
            } => {
                let state = if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                self.pointer_button(button, state, time);
            }
            HostEvent::Axis {
                horizontal,
                vertical,
                source,
                time,
            } => {
                let mut frame = AxisFrame::new(time);
                if let Some(source) = source.and_then(axis_source) {
                    frame = frame.source(source);
                }
                for (axis, scroll) in [(Axis::Horizontal, horizontal), (Axis::Vertical, vertical)] {
                    frame = add_axis(frame, axis, &scroll, self.host_scale());
                }
                self.pointer_axis(frame);
            }
            HostEvent::Relative {
                dx,
                dy,
                dx_unaccel,
                dy_unaccel,
                utime,
            } => {
                let under = self.x11_under(pointer.current_location());
                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta: (dx, dy).into(),
                        delta_unaccel: (dx_unaccel, dy_unaccel).into(),
                        utime,
                    },
                );
                pointer.frame(self);
            }
            HostEvent::PinchBegin { time, fingers } => pointer.gesture_pinch_begin(
                self,
                &GesturePinchBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    fingers,
                },
            ),
            HostEvent::PinchUpdate {
                time,
                dx,
                dy,
                scale,
                rotation,
            } => pointer.gesture_pinch_update(
                self,
                &GesturePinchUpdateEvent {
                    time,
                    delta: (dx, dy).into(),
                    scale,
                    rotation,
                },
            ),
            HostEvent::PinchEnd { time, cancelled } => pointer.gesture_pinch_end(
                self,
                &GesturePinchEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    cancelled,
                },
            ),
            HostEvent::SwipeBegin { time, fingers } => pointer.gesture_swipe_begin(
                self,
                &GestureSwipeBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    fingers,
                },
            ),
            HostEvent::SwipeUpdate { time, dx, dy } => pointer.gesture_swipe_update(
                self,
                &GestureSwipeUpdateEvent {
                    time,
                    delta: (dx, dy).into(),
                },
            ),
            HostEvent::SwipeEnd { time, cancelled } => pointer.gesture_swipe_end(
                self,
                &GestureSwipeEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    cancelled,
                },
            ),
            HostEvent::HoldBegin { time, fingers } => pointer.gesture_hold_begin(
                self,
                &GestureHoldBeginEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    fingers,
                },
            ),
            HostEvent::HoldEnd { time, cancelled } => pointer.gesture_hold_end(
                self,
                &GestureHoldEndEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    cancelled,
                },
            ),
            HostEvent::Resized { .. } | HostEvent::Frame | HostEvent::Close => {}
        }
    }

    fn key(&mut self, code: Keycode, state: KeyState, time: u32) {
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.input::<(), _>(
            self,
            code,
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    fn pointer_motion(&mut self, location: Point<f64, Logical>, time: u32) {
        let under = self.x11_under(location);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
        self.update_constraint();
    }

    fn pointer_button(&mut self, button: u32, state: ButtonState, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    fn pointer_axis(&mut self, frame: AxisFrame) {
        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    fn now(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn host_scale(&self) -> f64 {
        match self.backend.as_ref() {
            Some(Backend::Host(backend)) => backend.host.scale(),
            _ => 1.0,
        }
    }

    /// Activate a game's pointer lock or confinement once the pointer is over
    /// it, and mirror a lock on the host so the desktop pointer stays put.
    pub fn update_constraint(&mut self) {
        let pointer = self.seat.get_pointer().unwrap();
        let surface = pointer.current_focus().and_then(|focus| focus.wl_surface());
        let locked = surface.as_ref().is_some_and(|surface| {
            with_pointer_constraint(surface, &pointer, |constraint| match constraint {
                Some(constraint) => {
                    if !constraint.is_active() {
                        constraint.activate();
                    }
                    matches!(&*constraint, PointerConstraint::Locked(_))
                }
                None => false,
            })
        });
        if let Some(Backend::Host(backend)) = self.backend.as_mut() {
            backend.host.set_pointer_locked(locked);
        }
    }

    pub fn pointer_locked(&self) -> bool {
        let pointer = self.seat.get_pointer().unwrap();
        pointer
            .current_focus()
            .and_then(|focus| focus.wl_surface())
            .is_some_and(|surface| {
                with_pointer_constraint(&surface, &pointer, |constraint| {
                    constraint.is_some_and(|c| {
                        c.is_active() && matches!(&*c, PointerConstraint::Locked(_))
                    })
                })
            })
    }
}

fn axis_source(source: wl_pointer::AxisSource) -> Option<AxisSource> {
    Some(match source {
        wl_pointer::AxisSource::Wheel => AxisSource::Wheel,
        wl_pointer::AxisSource::Finger => AxisSource::Finger,
        wl_pointer::AxisSource::Continuous => AxisSource::Continuous,
        wl_pointer::AxisSource::WheelTilt => AxisSource::WheelTilt,
        _ => return None,
    })
}

/// Scroll distances are logical on the host; ours are physical pixels.
fn add_axis(mut frame: AxisFrame, axis: Axis, scroll: &AxisScroll, scale: f64) -> AxisFrame {
    if scroll.absolute != 0.0 {
        frame = frame.value(axis, scroll.absolute * scale);
    }
    if scroll.value120 != 0 {
        frame = frame.v120(axis, scroll.value120);
    } else if scroll.discrete != 0 {
        frame = frame.v120(axis, scroll.discrete * 120);
    }
    if scroll.stop {
        frame = frame.stop(axis);
    }
    frame
}
