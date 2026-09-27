#pragma once
#include <QObject>
#include <QWindow>
#include <cstdint>
namespace gamebuddy {
inline double window_scale(QObject *object) {
    auto *window = qobject_cast<QWindow *>(object);
    return window ? window->devicePixelRatio() : 1.0;
}
inline std::uint32_t window_id(QObject *object) {
    auto *window = qobject_cast<QWindow *>(object);
    return window ? static_cast<std::uint32_t>(window->winId()) : 0;
}
}
