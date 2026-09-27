#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qguiapplication.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;
        include!("cxx-qt-lib/core/qvector/qvector_QByteArray.h");
        type QVector_QByteArray = cxx_qt_lib::QVector<cxx_qt_lib::QByteArray>;
        #[namespace = "rust::cxxqtlib1"]
        fn qguiapplicationNew(args: &QVector_QByteArray) -> UniquePtr<QGuiApplication>;
    }
    unsafe extern "C++" {
        include!("QtWebEngineQuick/qtwebenginequickglobal.h");
        #[namespace = "QtWebEngineQuick"]
        fn initialize();
    }
}
pub fn initialize() {
    ffi::initialize();
}

/// Game metadata includes --platform, which Qt would interpret as its QPA
/// plugin selector. Qt receives argv[0] only; Rust and QML retain the original
/// application arguments. The CXX-Qt helper owns argv for the app's lifetime.
pub fn application() -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    let mut args = cxx_qt_lib::QVector::<cxx_qt_lib::QByteArray>::default();
    let program = std::env::args_os()
        .next()
        .unwrap_or_else(|| "gamebuddy".into());
    args.append(cxx_qt_lib::QByteArray::from(
        program.to_string_lossy().as_bytes(),
    ));
    ffi::qguiapplicationNew(&args)
}
