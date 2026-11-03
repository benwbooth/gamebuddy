import QtQuick

Rectangle {
    id: captions
    property string text: ""
    property bool active: false
    property bool lingering: false
    // When the words being spoken are known (local speech gives the whole
    // reply at once), the window follows the voice at about the pace it
    // speaks; otherwise it keeps the newest words of a streaming caption.
    property double startedAt: 0
    property real charsPerSecond: 15
    // Where the voice is in `text`, reported by the speech output; -1 when
    // it is not known and the pace estimate stands in.
    property var positionAt: null
    property double now: Date.now()
    Timer { interval: 150; repeat: true; running: captions.startedAt > 0 && captions.visible; onTriggered: captions.now = Date.now() }
    readonly property string excerpt: {
        if (text.length <= 240) return text
        let end = text.length
        if (startedAt > 0) {
            const reported = positionAt ? positionAt(now) : -1
            const spoken = reported >= 0 ? reported : Math.max(0, (now - startedAt) / 1000 * charsPerSecond)
            end = Math.min(text.length, Math.max(240, Math.round(spoken) + 120))
            const stop = text.indexOf(" ", end)
            if (end < text.length && stop >= 0) end = stop
        }
        const shown = text.slice(Math.max(0, end - 240), end)
        if (end - 240 <= 0) return shown + (end < text.length ? " …" : "")
        const space = shown.indexOf(" ")
        return "… " + (space >= 0 ? shown.slice(space + 1) : shown) + (end < text.length ? " …" : "")
    }
    visible: text.length > 0 && (active || lingering)
    width: parent ? Math.min(parent.width - 48, Math.max(780, Math.min(1280, parent.width * 0.55))) : 780
    height: captionLabel.implicitHeight + 26
    radius: 12
    color: "#eb101b25"
    border.color: "#6088a3b7"
    onTextChanged: { lingering = text.length > 0; expiry.restart() }
    onActiveChanged: if (!active) expiry.restart()
    Timer {
        id: expiry
        interval: 8000
        onTriggered: if (!captions.active) captions.lingering = false
    }
    Text {
        id: captionLabel
        objectName: "liveVoiceCaption"
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 13 }
        text: captions.excerpt
        textFormat: Text.PlainText
        color: "#f3f7fa"
        font.pixelSize: Math.max(22, Math.min(36, captions.parent ? captions.parent.height / 40 : 22))
        font.weight: Font.Medium
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        lineHeight: 1.15
    }
}
