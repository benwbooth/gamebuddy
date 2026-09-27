#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("QtWebEngineQuick/qtwebenginequickglobal.h");
        #[namespace = "QtWebEngineQuick"]
        fn initialize();
    }
}
pub fn initialize() {
    ffi::initialize();
}
