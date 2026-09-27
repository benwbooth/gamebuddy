mod controller;
mod webengine;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

fn main() {
    webengine::initialize();
    QQuickStyle::set_style(&QString::from("Basic"));
    let mut app = QGuiApplication::new();
    if let Some(mut app) = app.as_mut() {
        app.as_mut()
            .set_application_name(&QString::from("GameBuddy"));
        app.as_mut()
            .set_organization_name(&QString::from("GameBuddy"));
        app.as_mut()
            .set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    QGuiApplication::set_desktop_file_name(&QString::from("org.gamebuddy.GameBuddy"));
    let mut engine = QQmlApplicationEngine::new();
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let failure = failed.clone();
    let _creation_guard = engine.pin_mut().on_object_creation_failed(move |_, _| {
        failure.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    engine
        .pin_mut()
        .load(&QUrl::from("qrc:/qt/qml/GameBuddy/qml/Main.qml"));
    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
    app.pin_mut().exec();
}
