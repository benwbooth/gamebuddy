import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import QtMultimedia
import QtTextToSpeech
import QtWebEngine
import GameBuddy

ApplicationWindow {
    id: root
    width: 1240
    height: 860
    minimumWidth: 980
    minimumHeight: 680
    visible: Qt.application.arguments.indexOf("--workbench") >= 0
    title: "GameBuddy"
    color: "#0e1520"
    property var session: ({})
    property alias buddyObject: buddy
    property int page: 0
    property bool grabbing: false
    property bool autoSync: false
    property string captureError: ""
    property string smokePath: arg("--smoke-test")
    property string replayPath: arg("--replay")
    property bool editOverlay: false
    property bool widgetsEnabled: true
    property color ink: "#e6edf3"
    property color muted: "#91a3b6"
    property var selected: session.selected || null
    property var steps: session.guide ? session.guide.steps : []
    property var candidates: session.estimate ? session.estimate.candidates : []
    function arg(name) {
        let args = Qt.application.arguments
        let i = args.indexOf(name)
        return i >= 0 && i + 1 < args.length ? args[i + 1] : ""
    }
    function stepTitle(id) {
        for (let step of steps) if (step.id === id) return step.title
        return id
    }
    function time(ms) {
        return Math.floor(ms / 60000) + ":" + String(Math.floor(ms / 1000) % 60).padStart(2, "0")
    }
    function grabFrame() {
        if (buddy.busy || grabbing || !session.reference_count) return
        let output = replay.source.toString().length > 0 ? replayOutput : liveOutput
        if (output.contentRect.width <= 0 || output.contentRect.height <= 0) return
        buddy.match_sink(output.videoSink)
    }
    Buddy {
        id: buddy
        onState_jsonChanged: root.session = JSON.parse(state_json)
        Component.onCompleted: initialize()
        onAnswerChanged: { if (companion.spokenReplies && answer.length > 0) speech.say(answer) }
    }
    Companion {
        id: companion
        buddy: root.buddyObject
        session: root.session
        expanded: Qt.application.arguments.indexOf("--expanded") >= 0
        onCaptureRequested: { replay.stop(); replay.source = ""; root.captureError = ""; capture.active = !capture.active; root.autoSync = capture.active }
        onWorkbenchRequested: { root.visible = true; root.raise(); root.requestActivate() }
        onStopSpeaking: speech.stop()
        onWidgetsToggled: shown => root.widgetsEnabled = shown
        onArrangementChanged: active => { root.editOverlay = active; root.widgetsEnabled = true }
    }
    ScreenCapture {
        id: capture
        active: false
        onErrorOccurred: (error, errorString) => root.captureError = errorString
    }
    CaptureSession { screenCapture: capture; videoOutput: liveOutput }
    MediaPlayer {
        id: replay
        videoOutput: replayOutput
        source: root.replayPath.length > 0 ? "file://" + root.replayPath : ""
        onErrorOccurred: (error, errorString) => root.captureError = errorString
    }
    TextToSpeech { id: speech }
    Timer { interval: 2000; repeat: true; running: true; onTriggered: buddy.poll_widgets() }
    Timer {
        interval: 3000
        repeat: true
        running: root.autoSync && (capture.active || replay.playbackState === MediaPlayer.PlayingState)
        onTriggered: root.grabFrame()
    }
    FileDialog {
        id: fileDialog
        title: "Import a walkthrough"
        nameFilters: ["Walkthrough sources (*.json *.md *.txt *.mp4 *.mkv *.webm *.mov)"]
        onAccepted: buddy.import_file(root.session.game || "Faxanadu", selectedFile.toString())
    }
    FileDialog {
        id: frameDialog
        title: "Compare a gameplay screenshot"
        nameFilters: ["Images (*.png *.jpg *.jpeg)"]
        onAccepted: buddy.match_frame(selectedFile.toString())
    }
    FileDialog {
        id: replayDialog
        title: "Replay recorded gameplay"
        nameFilters: ["Video (*.mp4 *.mkv *.webm *.mov)"]
        onAccepted: { capture.active = false; replay.source = selectedFile; replay.play() }
    }
    Dialog {
        id: youtubeDialog
        title: "Import a YouTube walkthrough"
        anchors.centerIn: parent
        width: 560
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            width: parent.width
            Label { text: "Game" }
            GbField { id: youtubeGame; text: root.session.game || "Faxanadu"; Layout.fillWidth: true }
            Label { text: "Video URL" }
            GbField { id: youtubeUrl; placeholderText: "https://www.youtube.com/watch?v=…"; Layout.fillWidth: true }
            Label { text: "Downloads the video for local indexing. Author chapters become guide sections; compile sections to extract actions."; wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
        onAccepted: buddy.import_youtube(youtubeGame.text, youtubeUrl.text)
    }
    Dialog {
        id: textDialog
        title: "Add walkthrough text"
        anchors.centerIn: parent
        width: 680
        height: 560
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        ColumnLayout {
            anchors.fill: parent
            GbField { id: textGame; text: root.session.game || "Faxanadu"; Layout.fillWidth: true }
            GbField { id: textTitle; placeholderText: "Guide title"; Layout.fillWidth: true }
            ScrollView {
                Layout.fillWidth: true; Layout.fillHeight: true
                TextArea { id: textSource; placeholderText: "Paste a walkthrough. Markdown headings preserve chapters and substeps."; wrapMode: TextEdit.Wrap; selectByMouse: true }
            }
            CheckBox { id: compileText; text: "Compile steps with the reasoning model" }
        }
        onAccepted: buddy.import_text(textGame.text, textTitle.text, textSource.text, compileText.checked)
    }
    Window {
        id: osd
        transientParent: null
        width: Screen.width
        height: Screen.height
        x: 0; y: 0
        visible: root.widgetsEnabled && (!!root.session.current || (root.session.widgets || []).length > 0 || root.editOverlay)
        color: "transparent"
        flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | (root.editOverlay ? 0 : Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus)
        WebEngineView {
            id: widgetView
            anchors.fill: parent
            backgroundColor: "transparent"
            settings.localContentCanAccessRemoteUrls: true
            settings.localContentCanAccessFileUrls: false
            settings.javascriptCanOpenWindows: false
            function updateWidgets() {
                runJavaScript("window.gamebuddy && window.gamebuddy.render(" + JSON.stringify({widgets: root.session.widgets || [], current: root.session.current, layouts: root.session.layouts || {}, locked: !root.editOverlay}) + ")")
            }
            Component.onCompleted: loadHtml(buddy.widget_html())
            onLoadingChanged: loadRequest => { if (loadRequest.status === WebEngineView.LoadSucceededStatus) updateWidgets() }
            onJavaScriptConsoleMessage: (level, message, lineNumber, sourceID) => { if (message.startsWith("GAMEBUDDY_LAYOUT ")) buddy.save_widget_layout(message.slice(17)) }
            onPermissionRequested: permission => permission.deny()
            onNavigationRequested: request => { if (request.url.toString() !== "about:blank" && !request.url.toString().startsWith("data:")) request.action = WebEngineView.IgnoreRequest }
        }
        Connections { target: root; function onSessionChanged() { widgetView.updateWidgets() } function onEditOverlayChanged() { widgetView.updateWidgets() } }
    }
    ColumnLayout {
        id: workbenchContent
        anchors.fill: parent
        spacing: 0
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 78
            color: "#121d2a"
            RowLayout {
                anchors { fill: parent; leftMargin: 26; rightMargin: 26 }
                spacing: 20
                Rectangle {
                    width: 38; height: 38; radius: 11; color: "#86decb"
                    Text { anchors.centerIn: parent; text: "G"; font.pixelSize: 25; font.bold: true; color: "#14322f" }
                }
                ColumnLayout {
                    spacing: 2
                    Text { text: "GameBuddy"; color: root.ink; font.pixelSize: 21; font.weight: Font.DemiBold }
                    Text { text: "Your guide, in the game"; color: root.muted; font.pixelSize: 12 }
                }
                Item { Layout.fillWidth: true }
                Text { text: root.session.launch_context && root.session.launch_context.game_pid ? "LUNCHPAIL · PID " + root.session.launch_context.game_pid : "STANDALONE SESSION"; color: root.muted; font.pixelSize: 10; font.letterSpacing: 1 }
                GbButton { id: overlayButton; text: checked ? "Hide overlay" : "Show overlay"; checkable: true }
                GbButton { text: checked ? "Lock widgets" : "Arrange widgets"; checkable: true; onToggled: { root.editOverlay = checked; if (checked) overlayButton.checked = true } }
            }
        }
        RowLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
            Rectangle {
                Layout.preferredWidth: 192; Layout.fillHeight: true; color: "#111b27"
                ColumnLayout {
                    anchors { fill: parent; margins: 18 }
                    spacing: 10
                    Text { text: "WORKSPACE"; color: root.muted; font.pixelSize: 10; font.letterSpacing: 1.5; Layout.topMargin: 12; Layout.bottomMargin: 10 }
                    Repeater {
                        model: ["Session", "Walkthroughs", "Models"]
                        GbButton { required property int index; required property string modelData; text: modelData; accent: root.page === index; Layout.fillWidth: true; onClicked: root.page = index }
                    }
                    Item { Layout.fillHeight: true }
                    Rectangle { Layout.fillWidth: true; height: 1; color: "#2b3949" }
                    Text { text: "ACTIVE GAME"; color: root.muted; font.pixelSize: 10; Layout.topMargin: 10 }
                    Text { text: root.session.game || "Faxanadu"; color: root.ink; font.pixelSize: 17; Layout.fillWidth: true; wrapMode: Text.WordWrap }
                    Text { text: root.session.reference_count ? root.session.reference_count + " visual references" : "No references indexed"; color: root.muted; font.pixelSize: 11; Layout.fillWidth: true; wrapMode: Text.WordWrap }
                    Text { text: "Local vision + your LLM"; color: "#86decb"; font.pixelSize: 11; Layout.bottomMargin: 8 }
                }
            }
            StackLayout {
                currentIndex: root.page
                Layout.fillWidth: true; Layout.fillHeight: true
                // Session
                ScrollView {
                    clip: true
                    contentWidth: availableWidth
                    ColumnLayout {
                        width: parent.width
                        spacing: 18
                        anchors.margins: 24
                        Text { text: "Play with a little perspective."; color: root.ink; font.pixelSize: 26; Layout.leftMargin: 24; Layout.topMargin: 24 }
                        Text { text: "Match what you see to a source. Keep the next step close."; color: root.muted; font.pixelSize: 13; Layout.leftMargin: 24 }
                        RowLayout {
                            Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; spacing: 18
                            Rectangle {
                                Layout.fillWidth: true; Layout.preferredHeight: 265
                                color: "#080e16"; radius: 10; border.color: "#29394c"
                                Item {
                                    id: preview
                                    anchors { fill: parent; margins: 12 }
                                    VideoOutput {
                                        id: liveOutput
                                        anchors.centerIn: parent
                                        width: Math.min(parent.width, parent.height * (sourceRect.height > 0 ? sourceRect.width / sourceRect.height : 1.7778))
                                        height: width / (sourceRect.height > 0 ? sourceRect.width / sourceRect.height : 1.7778)
                                        visible: capture.active
                                    }
                                    VideoOutput {
                                        id: replayOutput
                                        anchors.centerIn: parent
                                        width: Math.min(parent.width, parent.height * (sourceRect.height > 0 ? sourceRect.width / sourceRect.height : 1.7778))
                                        height: width / (sourceRect.height > 0 ? sourceRect.width / sourceRect.height : 1.7778)
                                        visible: replay.source.toString().length > 0
                                    }
                                    Image { anchors.fill: parent; fillMode: Image.PreserveAspectFit; source: !capture.active && !replay.source.toString().length && root.session.frame_url ? root.session.frame_url + "?v=" + (root.session.estimate ? root.session.estimate.observed_at_ms : 0) : "" }
                                    Column {
                                        anchors.centerIn: parent; spacing: 14
                                        visible: !capture.active && !replay.source.toString().length && !root.session.frame_url
                                        Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Connect your game"; color: root.ink; font.pixelSize: 21 }
                                        Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Share a screen or compare a screenshot"; color: root.muted; font.pixelSize: 12 }
                                    }
                                }
                            }
                            Rectangle {
                                Layout.preferredWidth: 310; Layout.preferredHeight: 265; color: "#182534"; radius: 10
                                ColumnLayout {
                                    anchors { fill: parent; margins: 20 } spacing: 10
                                    Text { text: "CURRENT STEP"; color: "#86decb"; font.pixelSize: 10; font.letterSpacing: 1.4 }
                                    Text { text: root.session.current ? root.session.current.title : "Find your place"; color: root.ink; font.pixelSize: 20; font.weight: Font.DemiBold; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                    Text { text: root.session.current ? root.session.current.instructions : "Import a walkthrough, then match a frame or select a step to begin."; color: "#bdcddd"; font.pixelSize: 12; lineHeight: 1.3; wrapMode: Text.WordWrap; maximumLineCount: 7; elide: Text.ElideRight; Layout.fillWidth: true }
                                    Item { Layout.fillHeight: true }
                                    GbButton { text: "Read aloud"; enabled: !!root.session.current; onClicked: speech.say(root.session.current.title + ". " + root.session.current.instructions) }
                                }
                            }
                        }
                        Flow {
                            Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; spacing: 8
                            GbButton { text: capture.active ? "Stop sharing" : "Share screen"; onClicked: { replay.stop(); replay.source = ""; root.captureError = ""; capture.active = !capture.active } }
                            GbButton { text: "Replay video"; onClicked: replayDialog.open() }
                            GbButton { text: "Compare screenshot"; enabled: !buddy.busy && root.session.reference_count > 0; onClicked: frameDialog.open() }
                            GbButton { text: "Match frame"; enabled: !buddy.busy && root.session.reference_count > 0 && (capture.active || replay.source.toString().length > 0); onClicked: root.grabFrame() }
                            GbButton { text: checked ? "Auto sync on" : "Auto sync off"; checkable: true; onToggled: root.autoSync = checked }
                            GbButton { text: "Ask vision model"; enabled: !buddy.busy && root.candidates.length > 0; onClicked: buddy.compare_vision() }
                        }
                        Text { visible: root.captureError.length > 0; text: root.captureError; color: "#efbca6"; wrapMode: Text.WordWrap; Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24 }
                        Flow {
                            Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; spacing: 8
                            Repeater {
                                model: root.candidates
                                GbButton { required property var modelData; text: root.stepTitle(modelData.step_id) + " · " + root.time(modelData.reference_ms); onClicked: { buddy.select_step(modelData.step_id); root.page = 1 } }
                            }
                        }
                        Rectangle { Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; height: 1; color: "#2b3949" }
                        Text { text: "Talk through the next move"; color: root.ink; font.pixelSize: 18; Layout.leftMargin: 24 }
                        RowLayout {
                            Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24
                            GbField { id: question; Layout.fillWidth: true; placeholderText: "What should I do next?"; onAccepted: if (!buddy.busy && text.trim().length) buddy.ask(text) }
                            GbButton { text: "Ask"; accent: true; enabled: !buddy.busy && question.text.trim().length > 0; onClicked: buddy.ask(question.text) }
                            GbButton { text: "Read answer"; enabled: buddy.answer.length > 0; onClicked: speech.say(buddy.answer) }
                        }
                        Text { text: buddy.answer || "Ask for guidance, add a goal, choose a reference, or revise the outline. Hold the companion microphone button to speak."; color: buddy.answer ? root.ink : root.muted; font.pixelSize: 13; wrapMode: Text.WordWrap; textFormat: Text.PlainText; Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; Layout.bottomMargin: 24 }
                    }
                }
                // Walkthrough library
                ColumnLayout {
                    spacing: 14
                    Text { text: "Build your walkthrough"; color: root.ink; font.pixelSize: 26; Layout.leftMargin: 24; Layout.topMargin: 24 }
                    RowLayout {
                        Layout.leftMargin: 24; Layout.rightMargin: 24
                        GbButton { text: "YouTube video"; accent: true; enabled: !buddy.busy; onClicked: youtubeDialog.open() }
                        GbButton { text: "Find guide automatically"; enabled: !buddy.busy; onClicked: buddy.prepare_game() }
                        GbButton { text: "Paste text"; enabled: !buddy.busy; onClicked: textDialog.open() }
                        GbButton { text: "Import file"; enabled: !buddy.busy; onClicked: fileDialog.open() }
                        ComboBox {
                            Layout.fillWidth: true; textRole: "title"; valueRole: "id"; model: root.session.guides || []
                            onActivated: buddy.select_guide(currentValue)
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true; Layout.fillHeight: true; Layout.leftMargin: 24; Layout.rightMargin: 24; Layout.bottomMargin: 24; spacing: 20
                        ListView {
                            Layout.preferredWidth: 290; Layout.fillHeight: true; clip: true; spacing: 5; model: root.steps
                            ScrollBar.vertical: ScrollBar {}
                            delegate: Rectangle {
                                required property var modelData
                                width: ListView.view.width; height: stepLabel.implicitHeight + 26; radius: 7
                                color: root.selected && root.selected.id === modelData.id ? "#264139" : "#172332"
                                Text { id: stepLabel; anchors { left: parent.left; right: parent.right; top: parent.top; leftMargin: modelData.parent_id ? 28 : 12; rightMargin: 12; topMargin: 13 } text: modelData.title; color: root.ink; wrapMode: Text.WordWrap; font.pixelSize: 12 }
                                MouseArea { anchors.fill: parent; onClicked: buddy.select_step(modelData.id) }
                            }
                        }
                        ScrollView {
                            Layout.fillWidth: true; Layout.fillHeight: true; clip: true; contentWidth: availableWidth
                            ColumnLayout {
                                width: parent.width; spacing: 14
                                Text { text: root.selected ? root.selected.title : "Bring a source. Make it actionable."; color: root.ink; font.pixelSize: 22; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                Text { text: root.selected ? root.selected.instructions : "Import text, a local recording, or a YouTube walkthrough. GameBuddy preserves source links and builds an outline you can inspect."; color: "#bdcddd"; font.pixelSize: 13; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText }
                                Flow {
                                    Layout.fillWidth: true; spacing: 8
                                    GbButton { text: "I'm here"; accent: true; enabled: !!root.selected && !buddy.busy; onClicked: buddy.confirm_location() }
                                    GbButton { text: "Mark complete"; enabled: !!root.selected && !buddy.busy; onClicked: buddy.complete_step() }
                                    GbButton { text: "Open source"; enabled: !!root.selected; onClicked: Qt.openUrlExternally(buddy.source_url()) }
                                    GbButton { text: "Compile substeps"; enabled: !!root.selected && !buddy.busy; onClicked: buddy.compile_selected() }
                                }
                                Rectangle { Layout.fillWidth: true; height: 1; color: "#2b3949" }
                                Text { text: "VISUAL REFERENCES"; color: "#86decb"; font.pixelSize: 11 }
                                Text { text: "Sample the walkthrough, then compare live gameplay. Crop coordinates select the gameplay region in the reference video; share a tightly cropped game view for live matching."; color: root.muted; wrapMode: Text.WordWrap; Layout.fillWidth: true; font.pixelSize: 12 }
                                GbField { id: crop; Layout.fillWidth: true; text: '{"x":0,"y":0,"width":1,"height":1}' }
                                RowLayout {
                                    GbButton { text: "Build visual index"; enabled: !!root.session.guide && !buddy.busy; onClicked: buddy.build_index(10, crop.text) }
                                    GbButton { text: "Reset after reload"; enabled: !!root.session.guide && !buddy.busy; onClicked: buddy.reset_progress() }
                                }
                                Text { text: "SOURCE COVERAGE"; color: "#86decb"; font.pixelSize: 11; Layout.topMargin: 12 }
                                Text { text: root.session.guide ? root.session.guide.coverage_notes.join("\n\n") : "No source imported yet."; color: root.muted; font.pixelSize: 12; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText }
                            }
                        }
                    }
                }
                // Model configuration keeps credentials outside the database.
                ScrollView {
                    clip: true; contentWidth: availableWidth
                    ColumnLayout {
                        width: parent.width; spacing: 16
                        Text { text: "Choose the minds behind the guide"; color: root.ink; font.pixelSize: 26; Layout.leftMargin: 24; Layout.topMargin: 24 }
                        Text { text: "Reasoning and visual comparison are independent. Both default to your local Ollama model."; color: root.muted; font.pixelSize: 13; Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; wrapMode: Text.WordWrap }
                        Text { text: "Providers: ollama · open_ai_compatible · open_ai_responses · gemini"; color: "#86decb"; font.pixelSize: 12; Layout.leftMargin: 24 }
                        TextArea {
                            id: settingsEditor
                            Layout.fillWidth: true; Layout.preferredHeight: 355; Layout.leftMargin: 24; Layout.rightMargin: 24
                            text: JSON.stringify(JSON.parse(buddy.settings_json), null, 2)
                            color: root.ink; font.family: "monospace"; font.pixelSize: 13; selectByMouse: true; wrapMode: TextEdit.Wrap
                            background: Rectangle { color: "#172332"; radius: 10; border.color: "#344355" }
                        }
                        GbButton { text: "Save model configuration"; accent: true; enabled: !buddy.busy; Layout.leftMargin: 24; onClicked: buddy.save_settings(settingsEditor.text) }
                        Text { text: "api_key_env is the name of an environment variable, such as OPENAI_API_KEY. Keep the key itself outside this file. Cloud models receive the text and frames submitted to them."; color: root.muted; font.pixelSize: 12; wrapMode: Text.WordWrap; Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24 }
                    }
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true; Layout.preferredHeight: 44; color: "#14202e"
            RowLayout {
                anchors { fill: parent; leftMargin: 20; rightMargin: 20 } spacing: 10
                Rectangle { width: 7; height: 7; radius: 4; color: buddy.busy ? "#f4c779" : "#86decb" }
                Text { text: buddy.message; color: "#b5c6d6"; font.pixelSize: 12; elide: Text.ElideRight; Layout.fillWidth: true }
                Text { text: "PREVIEW 0.1"; color: root.muted; font.pixelSize: 10 }
            }
        }
    }
    Timer {
        interval: 1800
        running: root.smokePath.length > 0
        onTriggered: {
            const target = root.visible ? workbenchContent : companion.snapshotItem
            const accepted = target.grabToImage(function(result) {
                console.warn("GAMEBUDDY_SMOKE saved=" + result.saveToFile(root.smokePath) + " guides=" + (root.session.guides || []).length)
                Qt.quit()
            })
            if (!accepted) { console.warn("GAMEBUDDY_SMOKE capture rejected"); Qt.exit(1) }
        }
    }
    Component.onCompleted: {
        console.warn("GAMEBUDDY_READY game=" + (session.game || "") + " smoke=" + smokePath)
        if (replayPath.length > 0) replay.play()
    }
}
