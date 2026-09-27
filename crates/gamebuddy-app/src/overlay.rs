#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qrectf.h");
        type QRectF = cxx_qt_lib::QRectF;
        include!("gamebuddy_window.h");
        #[namespace = "gamebuddy"]
        unsafe fn window_id(window: *mut QObject) -> u32;
        #[namespace = "gamebuddy"]
        unsafe fn window_scale(window: *mut QObject) -> f64;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, active)]
        #[qproperty(QString, error)]
        #[qproperty(i32, focus_mode)]
        type Compositor = super::CompositorRust;
        #[qinvokable]
        unsafe fn attach(self: Pin<&mut Compositor>, window: *mut QObject);
        #[qinvokable]
        fn poll(self: Pin<&mut Compositor>, shown: bool, interactive: bool, rect: QRectF) -> i32;
    }
}
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;
pub struct CompositorRust {
    active: bool,
    error: QString,
    focus_mode: i32,
    scale: f64,
    #[cfg(target_os = "linux")]
    native: Option<gamebuddy_overlay::x11::Overlay>,
}
impl Default for CompositorRust {
    fn default() -> Self {
        Self {
            active: cfg!(target_os = "linux") && std::env::var_os("GAMEBUDDY_GAMESCOPE").is_some(),
            error: QString::default(),
            focus_mode: 0,
            scale: 1.0,
            #[cfg(target_os = "linux")]
            native: None,
        }
    }
}
impl ffi::Compositor {
    pub unsafe fn attach(mut self: Pin<&mut Self>, window: *mut cxx_qt::QObject) {
        if !*self.active() {
            return;
        }
        // SAFETY: QML passes its live QQuickWindow; helper checks the QObject type.
        let id = unsafe { ffi::window_id(window) };
        // SAFETY: same borrowed, type-checked live window.
        self.as_mut().rust_mut().scale = unsafe { ffi::window_scale(window) };
        #[cfg(target_os = "linux")]
        match gamebuddy_overlay::x11::Overlay::attach(id) {
            Ok(native) => self.as_mut().rust_mut().native = Some(native),
            Err(e) => self.set_error(QString::from(format!("Gamescope overlay: {e:#}"))),
        }
        #[cfg(not(target_os = "linux"))]
        let _ = id;
    }
    pub fn poll(
        mut self: Pin<&mut Self>,
        shown: bool,
        interactive: bool,
        rect: cxx_qt_lib::QRectF,
    ) -> i32 {
        let scale = self.rust().scale;
        #[cfg(target_os = "linux")]
        if let Some(native) = self.as_mut().rust_mut().native.as_mut() {
            let result = native.poll(
                shown,
                interactive,
                [rect.x(), rect.y(), rect.width(), rect.height()]
                    .map(|v| (v * scale).round() as i32),
            );
            let mode = native.mode() as i32;
            self.as_mut().set_focus_mode(mode);
            match result {
                Ok(toggle) => return toggle,
                Err(e) => self.set_error(QString::from(format!("Gamescope input: {e:#}"))),
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (shown, interactive, rect, scale);
        0
    }
}
