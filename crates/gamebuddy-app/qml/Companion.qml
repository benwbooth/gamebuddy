import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia

Item {
    id: companion
    required property var buddy
    required property var voice
    required property var session
    property var startup: null
    property bool expanded: false
    property bool recording: false
    property bool pendingRecording: false
    property bool spokenReplies: true
    property bool widgetsVisible: true
    property bool arranging: false
    property string microphoneError: ""
    property alias snapshotItem: visualRoot
    signal captureRequested()
    signal cropRequested()
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
    property bool realtimeCapturing: false
    property bool capturing: recording || voice.listening || realtimeCapturing
    property var widgetCatalog: []
    signal widgetVisibilityChanged(string id, bool shown)
    signal utteranceReady(string path)
    width: expanded ? 450 : Math.max(interactionHint.visible ? interactionHint.x + interactionHint.width + 8 : 80, compactStartup.visible ? compactStartup.x + compactStartup.width + 8 : 0)
    height: expanded ? 600 : Math.max(80, compactStartup.visible ? compactStartup.y + compactStartup.height + 8 : 0)
    onWidthChanged: if (embedded && parent && parent.width > 0 && width > 0) x = Math.max(0, Math.min(parent.width - width, x))
    onHeightChanged: if (embedded && parent && parent.height > 0 && height > 0) y = Math.max(0, Math.min(parent.height - height, y))

    onExpandedChanged: {
        if(!expanded)widgetTray.close()
        if (embedded) {
            x = Math.max(0, Math.min(parent.width-width, x))
            y = Math.max(0, Math.min(parent.height-height, y))
        }
    }
    function sendMessage() {
        const message = chatInput.text.trim()
        if (!message || buddy.busy || recording || voiceTurnPending) return
        if (buddy.ask(message)) chatInput.clear()
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
        color: companion.capturing ? "#bf665c" : "#162c35"
        border { width: 2; color: companion.capturing ? "#ffc4b9" : "#86decb" }
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
                        SequentialAnimation on opacity { running: companion.capturing || buddy.busy || buddy.preparing; loops: Animation.Infinite; NumberAnimation { to: 0.35; duration: 350 + index*90 } NumberAnimation { to: 1; duration: 350 + index*90 } }
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
    Rectangle {
        id: interactionHint
        visible: companion.embedded && !companion.expanded
        x: orb.x + orb.width + 10
        anchors.verticalCenter: orb.verticalCenter
        width: interactionHintContent.implicitWidth + 24
        height: 36; radius: 9
        color: "#ed162c35"; border.color: "#547f91a1"
        Row {
            id: interactionHintContent
            anchors.centerIn: parent; spacing: 10
            Text { text: "Shift+Tab"; color: "#a5f2dd"; font.pixelSize: 14; font.weight: Font.DemiBold }
            Text { text: "Interact"; color: "#e6f1f6"; font.pixelSize: 14 }
        }
        Accessible.role: Accessible.Button
        Accessible.name: "Interact with GameBuddy (Shift+Tab)"
        Accessible.onPressAction: companion.expanded = true
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: companion.expanded = true
        }
    }
    StartupStatus {
        id: compactStartup
        objectName: "companionStartup"
        progress: companion.startup
        visible: !companion.expanded && shown
        x: 8; y: 80
        width: Math.min(340, (companion.embedded && companion.parent ? companion.parent.width : Screen.desktopAvailableWidth) - 16)
        MouseArea { anchors.fill: parent; onClicked: companion.expanded = true }
    }
    ColumnLayout {
        visible: companion.expanded
        anchors { top: parent.top; left: parent.left; right: parent.right; bottom: parent.bottom; topMargin: 25; leftMargin: 24; rightMargin: 24; bottomMargin: 22 }
        spacing: 13
        RowLayout {
            Layout.leftMargin: 62; Layout.fillWidth: true; Layout.preferredHeight: 46
            Item {
                Layout.fillWidth: true; Layout.preferredHeight: 46
                ColumnLayout {
                    anchors.fill: parent; spacing: 1
                    Text { text: "GameBuddy"; color: "#e6f1f6"; font.pixelSize: 20; font.weight: Font.DemiBold }
                    Text { text: companion.session.game || "Your game companion"; color: "#91a9b9"; font.pixelSize: 14; Layout.fillWidth: true; elide: Text.ElideRight }
                }
                MouseArea {
                    id: titleDrag
                    objectName: "companionTitleDrag"
                    anchors.fill: parent
                    hoverEnabled: true; cursorShape: Qt.SizeAllCursor; preventStealing: true
                    property point scenePress
                    property point startPosition
                    property bool moved: false
                    onPressed: mouse => { scenePress=mapToItem(companion.parent,mouse.x,mouse.y);startPosition=Qt.point(companion.x,companion.y);moved=false }
                    onPositionChanged: mouse => {
                        if (!pressed) return
                        const p=mapToItem(companion.parent,mouse.x,mouse.y)
                        if (!moved && Math.hypot(p.x-scenePress.x,p.y-scenePress.y)>Qt.styleHints.startDragDistance) {
                            moved=true
                            if (!companion.embedded) companion.hostWindow.startSystemMove()
                        }
                        if (moved && companion.embedded) {
                            companion.x=Math.max(0,Math.min(companion.parent.width-companion.width,startPosition.x+p.x-scenePress.x))
                            companion.y=Math.max(0,Math.min(companion.parent.height-companion.height,startPosition.y+p.y-scenePress.y))
                        }
                    }
                    ToolTip.visible: containsMouse && !pressed
                    ToolTip.text: "Drag to move GameBuddy"
                }
            }
            GbButton { text: "−"; implicitWidth: 34; onClicked: companion.expanded = false; Accessible.name: "Minimize companion" }
        }
        StartupStatus {
            objectName: "companionExpandedStartup"
            progress: companion.startup
            Layout.fillWidth: true
        }
        Text {
            Layout.fillWidth: true; text: companion.recording ? "Listening… release when you're done." : companion.microphoneError || voice.error || (companion.realtimeEnabled ? companion.realtimeStatus || voice.status : voice.status)
            color: companion.microphoneError || voice.error ? "#f6b7a5" : "#a9c0cf"; font.pixelSize: 14; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
        }
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 62; radius: 13
            color: companion.recording ? "#724840" : "#254c49"; border.color: "#68bda9"
            Column {
                anchors.centerIn: parent; spacing: 3
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: companion.realtimeEnabled ? (voice.active ? "Voice chat active" : "Start voice chat") : companion.recording ? "Listening" : buddy.busy ? "Thinking…" : voice.active ? "Speak naturally" : "Hold to talk"; color: "#c7fff0"; font.pixelSize: 20; font.weight: Font.DemiBold }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: companion.realtimeEnabled ? "Talk naturally · change goals while you play" : voice.active ? "Pause when you're done · no click needed" : "Ask for help, a strategy, or a widget"; color: "#9fcec3"; font.pixelSize: 14 }
            }
            MouseArea { anchors.fill: parent; enabled: !buddy.busy && !companion.voiceTurnPending; onPressed: companion.startTalking(); onReleased: companion.stopTalking(); onCanceled: companion.stopTalking() }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: voice.active ? "Stop listening" : "Listen"; Layout.fillWidth: true; onClicked: voice.toggle_listening() }
            CheckBox {
                text: "Hands-free"; font.pixelSize: 14; checked: voice.armed; onToggled: voice.armed = checked
                palette.windowText: "#c7fff0"; palette.text: "#c7fff0"; palette.base: "#254c49"
                ToolTip.visible: hovered; ToolTip.text: "Listen automatically at game launch and when you unmute your microphone."
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Text {
            Layout.fillWidth: true
            text: companion.session.screen?.status === "live" ? "Watching game · " + (companion.session.screen.scene_error ? "visual analysis unavailable; screenshots ready" : companion.session.screen.scene?.fresh ? companion.session.screen.scene.summary : "interpreting the scene…") : "Screen " + (companion.session.screen?.status || "waiting") + " · " + (companion.session.screen?.error || buddy.message)
            color: "#a9c0cf"; font.pixelSize: 14; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
            }
            GbButton {text:"Crop…";enabled:!!companion.session.frame_url;onClicked:companion.cropRequested()}
        }
        ScrollView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true; contentWidth: availableWidth
            Column {
                width: parent.width; spacing: 10
                Text { width: parent.width; visible: buddy.transcript.length > 0; text: "You: " + buddy.transcript; color: "#91a9b9"; font.pixelSize: 14; wrapMode: Text.WordWrap; textFormat: Text.PlainText }
                Text { objectName: "companionChatReply"; width: parent.width; text: buddy.chat_error || buddy.answer || (buddy.busy && buddy.transcript ? "Thinking…" : "") || ((companion.session.navigation?.guidance?.status === "ready" && companion.session.current) ? companion.session.current.instructions : buddy.preparing ? "Preparing your walkthrough and visual references. The outline will fill in as sections are interpreted." : "Your outline is available over the game. Watching for a reliable location match before giving the next instruction."); color: buddy.chat_error ? "#f6b7a5" : "#e3edf5"; font.pixelSize: 16; lineHeight: 1.25; wrapMode: Text.WordWrap; textFormat: Text.PlainText }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            GbField {
                id: chatInput
                objectName: "companionChatInput"
                Layout.fillWidth: true
                placeholderText: "Message GameBuddy…"
                Accessible.name: "Message GameBuddy"
                onAccepted: companion.sendMessage()
            }
            GbButton {
                objectName: "companionChatSend"
                text: "Send"
                enabled: chatInput.text.trim().length > 0 && !buddy.busy && !companion.recording && !companion.voiceTurnPending
                onClicked: companion.sendMessage()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: companion.embedded ? (companion.syncActive ? "Pause watching" : "Resume watching") : "Share game"; Layout.fillWidth: true; onClicked: companion.captureRequested() }
            GbButton { text: companion.spokenReplies ? "Voice on" : "Voice off"; onClicked: { companion.spokenReplies = !companion.spokenReplies; if (!companion.spokenReplies) companion.stopSpeaking() } }
            GbButton { text: "Stop"; onClicked: companion.stopSpeaking() }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: "Browse guide"; Layout.fillWidth: true; enabled: !!companion.session.guide; onClicked: companion.widgetVisibilityChanged("guide-library",true) }
            GbButton { text: "Options"; onClicked: companion.workbenchRequested() }
        }
        RowLayout {
            Layout.fillWidth: true
            GbButton { text: companion.widgetsVisible ? "Hide widgets" : "Show widgets"; Layout.fillWidth: true; onClicked: { companion.widgetsVisible = !companion.widgetsVisible; companion.widgetsToggled(companion.widgetsVisible) } }
            GbButton { text: "Widgets…"; onClicked: widgetTray.open() }
        }
    }
    }
    Popup {
        id: widgetTray
        parent: companion
        x: 20; y: 90; width: companion.width-40
        height: Math.min(410, 150+companion.widgetCatalog.length*52)
        padding: 14
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle { color:"#162c35";radius:12;border.color:"#86decb" }
        contentItem: ColumnLayout {
            RowLayout {
                Text { text:"Your widgets";color:"#c7fff0";font.pixelSize:18;Layout.fillWidth:true }
                GbButton { text:"Done";onClicked:widgetTray.close() }
            }
            ScrollView {
                Layout.fillWidth:true;Layout.fillHeight:true;clip:true
                Column {
                    width: parent.width
                    Repeater {
                        model: companion.widgetCatalog
                        CheckDelegate {
                            id: widgetToggle
                            required property var modelData
                            width: widgetTray.availableWidth
                            height: 48
                            text:modelData.title
                            checked:companion.session.widget_visibility?.[modelData.id] ?? modelData.defaultVisible ?? true
                            palette.text:"#e3edf5";palette.windowText:"#e3edf5"
                            indicator: Rectangle {
                                x: parent.width-width-10;y:(parent.height-height)/2;width:22;height:22;radius:4
                                color:widgetToggle.checked?"#86decb":"#213f48";border.color:"#86decb"
                                Text { anchors.centerIn:parent;text:widgetToggle.checked?"✓":"";color:"#142130" }
                            }
                            background: Rectangle { color:widgetToggle.hovered?"#27434c":"transparent";radius:5 }
                            contentItem: Text { text:widgetToggle.text;color:"#e3edf5";font.pixelSize:14;verticalAlignment:Text.AlignVCenter;rightPadding:40;elide:Text.ElideRight }
                            onClicked:companion.widgetVisibilityChanged(modelData.id,checked)
                        }
                    }
                }
            }
            Text { text:"Dismissed widgets stay here. Use − to minimize a widget; drag its header or corner to arrange it.";color:"#a9c0cf";font.pixelSize:14;wrapMode:Text.WordWrap;Layout.fillWidth:true }
        }
    }
    Shortcut { sequence: "Escape"; enabled: !companion.embedded; onActivated: { companion.stopTalking(); companion.arranging = false; companion.arrangementChanged(false); companion.expanded = false } }
}
