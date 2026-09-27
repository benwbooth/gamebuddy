#pragma once
#include <QAudioSource>
#include <QAudioDevice>
#include <QCoreApplication>
#include <QMediaDevices>
#include <QPermissions>
#include <QIODevice>
#include <memory>

namespace gamebuddy {
// Only the Qt device/PCM boundary lives in C++; activation and VAD are Rust.
inline QAudioDevice selected_microphone() {
    const auto requested = qgetenv("GAMEBUDDY_AUDIO_SOURCE");
    if (requested.isEmpty()) return QMediaDevices::defaultAudioInput();
    for (const auto &device : QMediaDevices::audioInputs())
        if (device.id() == requested) return device;
    return {}; // Never silently record another device if an explicit ID vanished.
}
inline QString microphone_id() { return QString::fromUtf8(selected_microphone().id()); }
inline QString microphone_name() { return selected_microphone().description(); }
class Microphone {
public:
    std::unique_ptr<QAudioSource> source;
    QIODevice *stream = nullptr;
    QString failure;
    bool start() {
        stop();
        const auto permission = qApp->checkPermission(QMicrophonePermission{});
        if (permission == Qt::PermissionStatus::Undetermined) {
            qApp->requestPermission(QMicrophonePermission{}, [](const QPermission &) {});
            failure = QStringLiteral("Grant microphone access, then choose Listen.");
            return false;
        }
        if (permission == Qt::PermissionStatus::Denied) {
            failure = QStringLiteral("Microphone permission denied. Enable access in system settings.");
            return false;
        }
        const auto device = selected_microphone();
        QAudioFormat format;
        format.setSampleRate(16000); format.setChannelCount(1); format.setSampleFormat(QAudioFormat::Int16);
        if (device.isNull() || !device.isFormatSupported(format)) {
            failure = QStringLiteral("This microphone cannot capture 16 kHz mono audio. Hold to talk is still available.");
            return false;
        }
        source = std::make_unique<QAudioSource>(device, format);
        source->setBufferSize(16000); // at most half a second of 16-bit mono
        stream = source->start();
        if (!stream || source->error() != QtAudio::NoError) {
            failure = QStringLiteral("Could not open the microphone."); stop(); return false;
        }
        failure.clear(); return true;
    }
    void stop() { if (source) source->stop(); stream = nullptr; source.reset(); }
    QByteArray read() { return stream ? stream->readAll() : QByteArray{}; }
    QString error() const {
        if (source && source->error() != QtAudio::NoError) return QStringLiteral("Microphone capture interrupted.");
        return failure;
    }
};
inline std::unique_ptr<Microphone> microphone_new() { return std::make_unique<Microphone>(); }
}
