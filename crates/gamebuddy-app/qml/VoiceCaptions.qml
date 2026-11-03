import QtQuick

Rectangle {
    id: captions
    property string text: ""
    property bool active: false
    property bool lingering: false
    // How much of `text` has been spoken, in characters, when the voice
    // reports it: the words appear as they are said. -1 shows all of `text`
    // (a caption that already streams in as it is spoken).
    property int spokenChars: -1
    readonly property string shown: {
        if (spokenChars < 0 || spokenChars >= text.length) return text
        // Whole words only: finish the word being said.
        const space = text.indexOf(" ", spokenChars)
        return text.slice(0, space < 0 ? text.length : space)
    }
    // Three lines at most; older lines scroll up out of the box.
    readonly property int maxLines: 3
    visible: text.length > 0 && (active || lingering)
    width: parent ? Math.min(parent.width - 48, Math.max(780, Math.min(1280, parent.width * 0.55))) : 780
    height: Math.min(captionLabel.implicitHeight, captionLabel.lineSize * maxLines) + 26
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
    Item {
        anchors { fill: parent; margins: 13 }
        clip: true
        Text {
            id: captionLabel
            objectName: "liveVoiceCaption"
            readonly property real lineSize: lineCount > 0 ? implicitHeight / lineCount : font.pixelSize * 1.4
            anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
            text: captions.shown
            textFormat: Text.PlainText
            color: "#f3f7fa"
            font.pixelSize: Math.max(22, Math.min(36, captions.parent ? captions.parent.height / 40 : 22))
            font.weight: Font.Medium
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            lineHeight: 1.15
        }
    }
}
