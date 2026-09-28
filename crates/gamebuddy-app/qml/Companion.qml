import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia

Item {
    id: companion
    required property var buddy
    required property var voice
    required property var session
    property bool expanded: false
    property bool recording: false
    property bool pendingRecording: false
    property bool spokenReplies: true
    property bool widgetsVisible: true
    property bool arranging: false
    property string microphoneError: ""
    property alias snapshotItem: visualRoot
    signal captureRequested()
    signal workbenchRequested()
    signal stopSpeaking()
    signal widgetsToggled(bool shown)
    signal arrangementChanged(bool active)
    property var hostWindow
    property bool embedded: false
    property bool syncActive: false
    property bool voiceTurnPending: false
    property bool realtimeEnabled: false
    property string realtimeStatus: ""
    signal utteranceReady(string path)
    width: expanded ? 410 : 80
    height: expanded ? 480 : 80

    onExpandedChanged: {
        if (embedded) {
            x = Math.max(0, Math.min(parent.width-width, x))
            y = Math.max(0, Math.min(parent.height-height, y))
        }
    }
    function startTalking() {
        if(realtimeEnabled){if(!voice.active)voice.toggle_listening();return}
        if (buddy.busy || recording || voiceTurnPending) return
        microphoneError = ""
        stopSpeaking()
        voice.suspend_capture()
        recorder.outputLocation = buddy.voice_path()
        pendingRecording = true
        recorder.record()
    }
    function stopTalking() {
        if (recorder.recorderState === MediaRecorder.RecordingState) recorder.stop()
    }
    AudioInput { id: microphone }
    CaptureSession { audioInput: microphone; recorder: recorder }
    MediaRecorder {
        id: recorder
        mediaFormat { fileFormat: MediaFormat.Wave; audioCodec: MediaFormat.Wave }
        audioSampleRate: 16000
        audioChannelCount: 1
        onRecorderStateChanged: {
            companion.recording = recorderState === MediaRecorder.RecordingState
            if (recorderState === MediaRecorder.StoppedState && companion.pendingRecording) {
                companion.pendingRecording = false
                if (error === MediaRecorder.NoError) companion.utteranceReady(actualLocation.toString())
            }
        }
        onErrorOccurred: (error, errorString) => { companion.pendingRecording = false; companion.microphoneError = errorString }
    }
    Timer { interval: 45000; running: companion.recording; onTriggered: companion.stopTalking() }
    Item {
      id: visualRoot
      anchors.fill: parent
    Rectangle {
        id: panel
        visible: companion.expanded
        anchors.fill: parent
        anchors.margins: 4
        radius: 22
        color: "#f3132230"
        border.color: "#547f91a1"
    }
    Rectangle {
        id: orb
        x: 8; y: 8; width: 64; height: 64; radius: 32
        color: companion.recording || voice.listening ? "#bf665c" : "#162c35"
        border { width: 2; color: companion.recording || voice.listening ? "#ffc4b9" : "#86decb" }
        Rectangle {
            anchors.centerIn: parent
            width: 45; height: 45; radius: 22
            color: "#243d46"
            Row {
                anchors.centerIn: parent; spacing: 4
                Repeater {
                    model: 4
                    Rectangle {
                        required property int index
                        width: 4; height: index === 1 || index === 2 ? 21 : 11; radius: 2
                        y: (21-height)/2; color: "#a5f2dd"
                        SequentialAnimation on opacity { running: companion.recording || voice.listening || buddy.busy || buddy.preparing; loops: Animation.Infinite; NumberAnimation { to: 0.35; duration: 350 + index*90 } NumberAnimation { to: 1; duration: 350 + index*90 } }
                    }
                }
            }
        }
        MouseArea {
            id: orbMouse
            anchors.fill: parent
            hoverEnabled: true
            property point pressedAt
            property point scenePress
            property point startPosition
            property bool moved: false
            property bool held: false
            onPressed: mouse => { pressedAt = Qt.point(mouse.x,mouse.y); scenePress = mapToItem(companion.parent, mouse.x, mouse.y); startPosition = Qt.point(companion.x, companion.y); moved = false; held = false }
            onPositionChanged: mouse => {
                if (pressed && !held) {
                    const p = mapToItem(companion.parent, mouse.x, mouse.y)
                    if (Math.hypot(p.x-scenePress.x,p.y-scenePress.y)>8) {
                        if (!moved && !companion.embedded) companion.hostWindow.startSystemMove()
                        moved = true
                    }
                    if (moved && companion.embedded) {
                        companion.x = Math.max(0, Math.min(companion.parent.width-companion.width, startPosition.x+p.x-scenePress.x))
                        companion.y = Math.max(0, Math.min(companion.parent.height-companion.height, startPosition.y+p.y-scenePress.y))
                    }
                }
            }
            onPressAndHold: { if (!moved) { held = true; companion.startTalking() } }
            onReleased: { if (held) companion.stopTalking(); else if (!moved) companion.expanded = !companion.expanded }
            onCanceled: companion.stopTalking()
            ToolTip.visible: containsMouse && !pressed
            ToolTip.text: voice.status + "\nClick to expand · Drag to move · Hold to talk"
        }
        Rectangle { visible: buddy.preparing; width: 10; height: 10; radius: 5; x: 49; y: 51; color: "#efc783"; border.color: "#132230"; border.width: 2 }
        Rectangle { visible: voice.active; width: 12; height: 12; radius: 6; x: 2; y: 49; color: voice.listening ? "#ffbdad" : "#efc783"; border.color: "#132230"; border.width: 2; Accessible.name: voice.status }
    }
    ColumnLayout {
        visible: companion.expanded
        anchors { top: parent.top; left: parent.left; right: parent.right; bottom: parent.bottom; topMargin: 25; leftMargin: 24; rightMargin: 24; bottomMargin: 22 }
        spacing: 13
        RowLayout {
            Layout.leftMargin: 62; Layout.fillWidth: true; Layout.preferredHeight: 46
            ColumnLayout {
                spacing: 1
                Text { text: "GameBuddy"; color: "#e6f1f6"; font.pixelSize: 18; font.weight: Font.DemiBold }
                Text { text: companion.session.game || "Your game companion"; color: "#91a9b9"; font.pixelSize: 12 }
            }
            Item { Layout.fillWidth: true }
            GbButton { text: "−"; implicitWidth: 34; onClicked: companion.expanded = false; Accessible.name: "Minimize companion" }
        }
        Text {
            Layout.fillWidth: true; text: companion.recording ? "Listening… release when you're done." : companion.microphoneError || voice.error || (companion.realtimeEnabled ? companion.realtimeStatus || voice.status : voice.status)
            color: companion.microphoneError || voice.error ? "#f6b7a5" : "#a9c0cf"; font.pixelSize: 12; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
        }
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 62; radius: 13
            color: companion.recording ? "#724840" : "#254c49"; border.color: "#68bda9"
            Column {
                anchors.centerIn: parent; spacing: 3
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: companion.realtimeEnabled ? (voice.active ? "Voice chat active" : "Start voice chat") : companion.recording ? "Listening" : buddy.busy ? "Thinking…" : "Hold to talk"; color: "#c7fff0"; font.pixelSize: 17; font.weight: Font.DemiBold }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: companion.realtimeEnabled ? "Talk naturally · change goals while you play" : "Ask for help, a strategy, or a widget"; color: "#9fcec3"; font.pixelSize: 11 }
            }
            MouseArea { anchors.fill: parent; enabled: !buddy.busy && !companion.voiceTurnPending; onPressed: companion.startTalking(); onReleased: companion.stopTalking(); onCanceled: companion.stopTalking() }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: voice.active ? "Stop listening" : "Listen"; Layout.fillWidth: true; onClicked: voice.toggle_listening() }
            CheckBox {
                text: "On unmute"; checked: voice.armed; onToggled: voice.armed = checked
                palette.windowText: "#c7fff0"; palette.text: "#c7fff0"; palette.base: "#254c49"
                ToolTip.visible: hovered; ToolTip.text: "Start listening when this microphone is unmuted in the system."
            }
        }
        Text { Layout.fillWidth: true; text: buddy.message; color: "#a9c0cf"; font.pixelSize: 11; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight }
        ScrollView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true; contentWidth: availableWidth
            Column {
                width: parent.width; spacing: 10
                Text { width: parent.width; visible: buddy.transcript.length > 0; text: "You: " + buddy.transcript; color: "#91a9b9"; font.pixelSize: 12; wrapMode: Text.WordWrap; textFormat: Text.PlainText }
                Text { width: parent.width; text: buddy.answer || (companion.session.current ? companion.session.current.instructions : buddy.preparing ? "Preparing your walkthrough and visual references. The outline will fill in as sections are interpreted." : "Your outline is available over the game. Watching for a reliable location match before giving the next instruction."); color: "#e3edf5"; font.pixelSize: 14; lineHeight: 1.25; wrapMode: Text.WordWrap; textFormat: Text.PlainText }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: companion.embedded ? (companion.syncActive ? "Pause sync" : "Resume sync") : "Share game"; Layout.fillWidth: true; onClicked: companion.captureRequested() }
            GbButton { text: companion.spokenReplies ? "Voice on" : "Voice off"; onClicked: { companion.spokenReplies = !companion.spokenReplies; if (!companion.spokenReplies) companion.stopSpeaking() } }
            GbButton { text: "Stop"; onClicked: companion.stopSpeaking() }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: companion.widgetsVisible ? "Hide widgets" : "Show widgets"; Layout.fillWidth: true; onClicked: { companion.widgetsVisible = !companion.widgetsVisible; companion.widgetsToggled(companion.widgetsVisible) } }
            GbButton { text: companion.arranging ? "Lock" : "Arrange"; onClicked: { companion.arranging = !companion.arranging; companion.arrangementChanged(companion.arranging) } }
            GbButton { text: "Options"; onClicked: companion.workbenchRequested() }
        }
    }
    }
    Shortcut { sequence: "Escape"; enabled: !companion.embedded; onActivated: { companion.stopTalking(); companion.arranging = false; companion.arrangementChanged(false); companion.expanded = false } }
}
