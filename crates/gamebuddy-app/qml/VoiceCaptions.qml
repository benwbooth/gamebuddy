import QtQuick

Rectangle {
    id: captions
    property string text: ""
    property bool active: false
    property bool lingering: false
    // Keep a readable moving window while the complete answer stays in chat.
    readonly property string excerpt: {
        if (text.length <= 240) return text
        const tail = text.slice(-240), space = tail.indexOf(" ")
        return "… " + (space >= 0 ? tail.slice(space + 1) : tail)
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
