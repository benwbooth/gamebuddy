use cxx_qt_build::{CxxQtBuilder, QmlModule};
fn main() {
    println!("cargo:rerun-if-changed=qml");
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("GameBuddy").qml_files([
        "qml/Main.qml",
        "qml/GbButton.qml",
        "qml/GbField.qml",
        "qml/Companion.qml",
        "qml/RealtimePanel.qml",
        "qml/CropEditor.qml",
    ]))
    .file("src/controller.rs")
    .file("src/overlay.rs")
    .file("src/voice.rs")
    .file("src/realtime.rs")
    .file("src/webengine.rs");
    // SAFETY: add a header search path only; preserve CXX-Qt's compiler flags.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.include("include");
        })
    };
    builder
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("Multimedia")
        .qt_module("TextToSpeech")
        .qt_module("WebEngineQuick")
        .build();
}
