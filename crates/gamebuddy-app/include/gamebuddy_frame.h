#pragma once
#include <QObject>
#include <QImage>
#include <QString>
#include <QVideoSink>
#include <QVideoFrame>

namespace gamebuddy {
// Qt owns the sink and frame. Copy only the sampled image; this also works
// while the workbench window is hidden and has no rendered preview surface.
inline bool save_video_frame(QObject *object, const QString &path) {
    auto *sink = qobject_cast<QVideoSink *>(object);
    if (!sink) return false;
    const QImage image = sink->videoFrame().toImage();
    if (image.isNull()) return false;
    return image.scaled(960, 720, Qt::KeepAspectRatio, Qt::SmoothTransformation).save(path, "PNG");
}
}
