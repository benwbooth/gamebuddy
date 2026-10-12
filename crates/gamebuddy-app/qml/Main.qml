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
    property var appArguments: JSON.parse(buddy.launch_arguments())
    width: 1240
    height: 860
    minimumWidth: 980
    minimumHeight: 680
    visible: !compositor.active && root.appArguments.indexOf("--workbench") >= 0
    title: "GameBuddy"
    color: "#0e1520"
    property var session: ({})
    property alias buddyObject: buddy
    property int page: 0
    property bool grabbing: false
    property bool autoSync: compositor.active
    onAutoSyncChanged: if(session.data_dir)buddy.set_screen_capture(autoSync)
    property bool overlayOptions: false
    property bool cropOpen: false
    property bool overlayHidden: false
    property string queuedVoice: ""
    property bool voiceTurnRunning: false
    property bool realtimeEnabled: JSON.parse(buddy.settings_json).speech === "codex_realtime"
    property bool geminiChosen: JSON.parse(buddy.settings_json).speech === "gemini_live"
    // When Google's service fails, keep answering with the local voice and
    // the selected assistant, and try Gemini again after a pause.
    property bool geminiFallback: false
    readonly property bool geminiEnabled: geminiChosen && !geminiFallback
    // Switching backends from the companion starts each one fresh.
    onGeminiChosenChanged: { geminiFallback = false; liveVoice.failed = false }
    onRealtimeEnabledChanged: if (realtimeEnabled) realtimeVoice.failed = false
    // A new Gemini voice or model takes effect by reconnecting the session.
    readonly property string geminiVoiceChoice: JSON.stringify(JSON.parse(buddy.settings_json).live_voice || {})
    onGeminiVoiceChoiceChanged: if (liveVoice.started) { liveVoice.shutdown("Switching Gemini voice…"); liveVoice.update() }
    // Either live backend owns listening and speech; local audio stays quiet.
    readonly property bool liveVoiceEnabled: realtimeEnabled || geminiEnabled
    property bool duckGameAudio: JSON.parse(buddy.settings_json).duck_game_audio !== false
    property string captureError: ""
    property string tokenAlert: ""
    Timer {
        interval: 120000; running: root.geminiFallback
        onTriggered: { liveVoice.failed = false; root.geminiFallback = false }
    }
    function noteOutOfTokens(text) {
        if (!text || !text.startsWith("Out of tokens:") || text === root.tokenAlert) return
        root.tokenAlert = text
        // The live voice may be the thing that ran out; say it locally.
        speech.say("Heads up: " + text.replace(/^Out of tokens:\s*/, "").split(".")[0] + ".")
    }
    property string smokePath: arg("--smoke-test")
    property string replayPath: arg("--replay")
    property bool editOverlay: false
    property bool widgetsEnabled: true
    property string widgetRegions: "[]"
    property var widgetCatalog: {
        let items=[]
        items.push({id:"last-seen-screen",title:"Last screenshot seen"})
        if(session.guide)items.push({id:"walkthrough-outline",title:"Walkthrough"})
        if(session.guide)items.push({id:"run-journal",title:"Run journal · notes & screenshots",defaultVisible:false})
        if(session.guide)items.push({id:"guide-library",title:"Guide library · pictures, maps & sources",defaultVisible:false})
        if(session.guide)items.push({id:"navigation-arrow",title:"Navigation arrow"})
        if(!session.guide && (session.navigation?.guidance || session.current))items.push({id:"current-step",title:"Next action"})
        if(session.navigation?.map)items.push({id:"location-map",title:"Location map"})
        return items.concat(session.widgets || [])
    }
    property color ink: "#e6edf3"
    property color muted: "#91a3b6"
    property var selected: session.selected || null
    property var steps: session.guide ? session.guide.steps : []
    property var candidates: session.estimate ? session.estimate.candidates : []
    function arg(name) {
        let args = root.appArguments
        let i = args.indexOf(name)
        return i >= 0 && i + 1 < args.length ? args[i + 1] : ""
    }
    function releaseOverlay() {
        companion.stopTalking(); companion.expanded = false; companion.arranging = false
        root.overlayOptions = false; root.editOverlay = false
        root.cropOpen = false
        if (!compositor.active) root.visible = false
    }
    function overlayAction(action) {
        if (action === 1) {
            if (overlayHidden) { overlayHidden = false; companion.expanded = true }
            else if (companion.expanded || editOverlay || overlayOptions || cropOpen) releaseOverlay()
            else companion.expanded = true
        } else if (action === 2) {
            overlayHidden = !overlayHidden
            releaseOverlay()
        } else if (action === 3) { overlayHidden = false }
        else if (action === 4) { overlayHidden = true; releaseOverlay() }
        else if (action === 5) { overlayHidden = false; companion.expanded = true }
        else if (action === 6) releaseOverlay()
        else if (action === 7) {
            overlayHidden = false; root.page = 1
            if (compositor.active) root.overlayOptions = true
            else { root.visible = true; root.raise(); root.requestActivate() }
        } else if (action === 8) {
            overlayHidden = false; companion.expanded = true
            root.overlayOptions = false
            companion.arranging = true; root.editOverlay = true; root.widgetsEnabled = true
        }
    }
    function stepTitle(id) {
        for (let step of steps) if (step.id === id) return step.title
        return id
    }
    function time(ms) {
        return Math.floor(ms / 60000) + ":" + String(Math.floor(ms / 1000) % 60).padStart(2, "0")
    }
    function grabFrame() {
        if (buddy.matching || grabbing) return
        buddy.set_screen_capture(true)
        if (compositor.active && replay.source.toString().length === 0) { buddy.match_game_frame(); return }
        let output = replay.source.toString().length > 0 ? replayOutput : liveOutput
        if (output.contentRect.width <= 0 || output.contentRect.height <= 0) return
        buddy.match_sink(output.videoSink)
    }
    function openCrop() {
        const preview=JSON.parse(buddy.crop_preview())
        if(preview.error){buddy.message=preview.error;return}
        cropEditor.configure(preview);root.overlayHidden=false;root.cropOpen=true
        if(!compositor.active){osd.raise();osd.requestActivate()}
    }
    Buddy {
        id: buddy
        onState_jsonChanged: root.session = JSON.parse(state_json)
        onMap_preview_ready: (id, preview) => widgetView.runJavaScript("gamebuddy.mapPreview("+JSON.stringify(id)+","+JSON.stringify(preview)+")")
        Component.onCompleted: initialize()
        onChat_errorChanged: root.noteOutOfTokens(chat_error)
        onMessageChanged: root.noteOutOfTokens(message)
        onAnswerChanged: { if (!root.liveVoiceEnabled && companion.spokenReplies && answer.length > 0) speech.say(answer) }
        onVoice_command: action => root.overlayAction(action)
        onBusyChanged: if (!busy) root.voiceTurnRunning = false
    }
    Compositor {
        id: compositor
        onFocus_modeChanged: if(root.appArguments.indexOf("--ui-trace")>=0)console.warn("GAMEBUDDY_UI_FOCUS mode="+focus_mode)
    }
    GameAudio {
        id: gameAudio
        onStatusChanged: if(root.appArguments.indexOf("--voice-trace")>=0)console.warn("GAMEBUDDY_AUDIO "+status)
    }
    Timer {
        interval:100;repeat:true;running:true
        onTriggered:gameAudio.poll(root.arg("--game-pid"),
            realtimeVoice.conversationActive || liveVoice.conversationActive || voice.speaking || companion.recording || companion.pendingRecording
            || root.voiceTurnRunning || root.queuedVoice.length>0 || speech.speaking,
            root.duckGameAudio)
    }
    RealtimePanel {
        id: realtimeVoice
        parent: compositor.active ? overlayScene : companionWindow.contentItem
        buddy: root.buddyObject; voice: voice; session: root.session
        enabledBackend: root.realtimeEnabled; spokenReplies: companion.spokenReplies
        automaticSession: compositor.active
        localConversationActive: buddy.busy || root.voiceTurnRunning || root.queuedVoice.length>0 || companion.recording || companion.pendingRecording || speech.speaking
        trace: root.appArguments.indexOf("--voice-trace") >= 0
        onTranscript: text => buddy.realtime_transcript(text)
        onAnswer: text => buddy.answer = text
        onOverlayAction: action => root.overlayAction(action)
    }
    LiveVoicePanel {
        id: liveVoice
        parent: compositor.active ? overlayScene : companionWindow.contentItem
        buddy: root.buddyObject; voice: voice; session: root.session
        contextSource: realtimeVoice
        commentary: companion.liveCommentary
        enabledBackend: root.geminiEnabled; spokenReplies: companion.spokenReplies
        automaticSession: compositor.active
        trace: root.appArguments.indexOf("--voice-trace") >= 0
        onTranscript: text => buddy.realtime_transcript(text)
        onAnswer: text => buddy.answer = text
        onOutOfTokens: message => root.noteOutOfTokens(message)
        onUnavailable: message => {
            root.geminiFallback = true
            if (message.length && !root.tokenAlert.startsWith("Out of tokens:"))
                root.tokenAlert = "Voice unavailable: Gemini Live is not responding (" + message + "). GameBuddy is using the local voice and will try Gemini again in 2 minutes."
        }
    }
    VoiceInput {
        id: voice
        armed: compositor.active && root.appArguments.indexOf("--no-listen") < 0
        onActiveChanged: if (active) root.overlayHidden = false
        onListeningChanged: if (root.appArguments.indexOf("--voice-trace") >= 0) console.warn("GAMEBUDDY_MIC_CAPTURE " + listening)
    }
    Timer {
        interval: 40; repeat: true; running: true
        onTriggered: {
            const path = voice.poll(buddy.voice_directory(), buddy.busy || root.voiceTurnRunning || root.queuedVoice.length > 0,
                speech.busy,
                companion.recording || companion.pendingRecording, root.liveVoiceEnabled)
            if (path.length) root.queuedVoice = path
            if (root.queuedVoice.length && !buddy.busy) {
                const ready = root.queuedVoice
                root.queuedVoice = ""; root.voiceTurnRunning = true
                buddy.voice_turn(ready)
            }
        }
    }
    Window {
        id: companionWindow
        transientParent: null
        visible: !compositor.active && !root.overlayHidden
        title: "GameBuddy companion"
        color: "transparent"
        x: 36; y: 90
        width: companion.width; height: companion.height
        flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
        function keepOnScreen() {
            if (!visible) return
            x = Math.max(Screen.virtualX, Math.min(x, Screen.virtualX + Screen.desktopAvailableWidth-width))
            y = Math.max(Screen.virtualY, Math.min(y, Screen.virtualY + Screen.desktopAvailableHeight-height))
        }
        onWidthChanged: Qt.callLater(keepOnScreen)
        onHeightChanged: Qt.callLater(keepOnScreen)
    }
    Companion {
        id: companion
        parent: compositor.active ? overlayScene : companionWindow.contentItem
        hostWindow: companionWindow
        embedded: compositor.active
        trace: root.appArguments.indexOf("--ui-trace") >= 0
        syncActive: root.autoSync
        startup: JSON.parse(buddy.startup_json)
        preparation: JSON.parse(buddy.preparation_json)
        tokenAlert: root.tokenAlert
        onTokenAlertDismissed: root.tokenAlert = ""
        onPreparationRetryRequested: buddy.prepare_game()
        x: compositor.active ? 36 : 0
        y: compositor.active ? 90 : 0
        z: 10
        buddy: root.buddyObject
        voice: voice
        realtimeEnabled: root.liveVoiceEnabled
        realtimeStatus: root.geminiEnabled ? liveVoice.status : realtimeVoice.status
        realtimeCapturing: root.geminiEnabled ? liveVoice.capturing : realtimeVoice.capturing
        realtimeThinking: root.geminiEnabled ? liveVoice.started && liveVoice.workPending : realtimeVoice.started && (realtimeVoice.workPending || realtimeVoice.backendBusy || (realtimeVoice.waitingForReply && !realtimeVoice.assistantSpeaking && !realtimeVoice.userSpeaking))
        spokenReplies: root.appArguments.indexOf("--quiet") < 0
        widgetCatalog: root.widgetCatalog
        onWidgetVisibilityChanged: (id,shown) => { buddy.set_widget_visible(id,shown);if(shown){root.widgetsEnabled=true;companion.widgetsVisible=true} }
        voiceTurnPending: root.queuedVoice.length > 0 || root.voiceTurnRunning
        onUtteranceReady: path => root.queuedVoice = path
        session: root.session
        expanded: root.appArguments.indexOf("--expanded") >= 0
        onCaptureRequested: { replay.stop(); replay.source = ""; root.captureError = ""; if (compositor.active) root.autoSync = !root.autoSync; else { capture.active = !capture.active; root.autoSync = capture.active } }
        onCropRequested: root.openCrop()
        onWorkbenchRequested: { if (compositor.active) root.overlayOptions = true; else { root.visible = true; root.raise(); root.requestActivate() } }
        onStopSpeaking: { speech.stop(); realtimeVoice.stopSpeaking(); liveVoice.stopSpeaking() }
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
    SpeechOutput {
        id: speech
        directory: root.session.data_dir || ""
        trace: root.appArguments.indexOf("--voice-trace") >= 0
    }
    Timer { interval: 2000; repeat: true; running: true; onTriggered: buddy.poll_widgets() }
    Timer { interval: 100; repeat: true; running: true; onTriggered: buddy.poll_startup() }
    Timer {
        interval: 16; repeat: true; running: root.appArguments.indexOf("--ui-trace") >= 0
        property double previous: 0
        onTriggered: {
            const now = Date.now()
            if (previous && now-previous > 100) console.warn("GAMEBUDDY_UI_STALL ms="+(now-previous))
            previous = now
        }
    }
    Timer {
        interval: 100
        repeat: true
        running: root.autoSync && (compositor.active || capture.active || replay.playbackState === MediaPlayer.PlayingState)
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
        opacity: root.overlayHidden ? 0 : 1
        transientParent: null
        width: Screen.width
        height: Screen.height
        x: 0; y: 0
        visible: root.cropOpen || compositor.active || (root.widgetsEnabled && (!!root.session.guide || (root.session.widgets || []).length > 0 || root.editOverlay))
        color: "transparent"
        flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | (root.cropOpen || root.editOverlay || companion.expanded || compositor.active ? 0 : Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus)
        Component.onCompleted: { if (compositor.active) compositor.attach(osd) }
        Item { id: overlayScene; anchors.fill: parent }
        NavigationArrow {
            parent: overlayScene
            z: 24
            enabled: root.widgetsEnabled && !root.cropOpen && !root.overlayOptions && (root.session.widget_visibility?.["navigation-arrow"] ?? true)
            cue: root.session.navigation?.direction || null
            trace: root.appArguments.indexOf("--navigation-trace") >= 0
            occupiedRects: {
                let boxes=widgetView.visible ? JSON.parse(root.widgetRegions) : []
                if (companion.embedded) boxes.push([companion.x,companion.y,companion.width,companion.height])
                if (liveCaptions.visible) boxes.push([liveCaptions.x,liveCaptions.y,liveCaptions.width,liveCaptions.height])
                return boxes
            }
        }
        VoiceCaptions {
            id: liveCaptions
            parent: overlayScene
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 32
            z: 25
            text: root.geminiEnabled ? liveVoice.captionText : realtimeVoice.captionText
            active: root.geminiEnabled ? liveVoice.captionStreaming || liveVoice.assistantSpeaking : realtimeVoice.captionStreaming || realtimeVoice.assistantSpeaking
        }
        WebEngineView {
            id: widgetView
            parent: overlayScene
            visible: root.widgetsEnabled && (!!root.session.screen || !!root.session.guide || (root.session.widgets || []).length > 0 || root.editOverlay)
            enabled: compositor.active || root.editOverlay || companion.expanded
            anchors.fill: parent
            backgroundColor: "transparent"
            settings.localContentCanAccessRemoteUrls: true
            settings.localContentCanAccessFileUrls: false
            settings.javascriptCanOpenWindows: false
            function updateWidgets() {
                runJavaScript("window.gamebuddy && window.gamebuddy.render(" + JSON.stringify({screen:root.session.screen, seen_screen:root.session.seen_screen, widgets: root.session.widgets || [], current: root.session.current, navigation: root.session.navigation, guide:root.session.guide, journal:root.session.journal, progress:root.session.progress, estimate:root.session.estimate, preparation:root.session.preparation, reference_count:root.session.reference_count, layouts: root.session.layouts || {}, widget_visibility:root.session.widget_visibility, locked: false}) + ")")
            }
            Component.onCompleted: loadHtml(buddy.widget_html())
            onLoadingChanged: loadRequest => { if (loadRequest.status === WebEngineView.LoadSucceededStatus) updateWidgets() }
            onJavaScriptConsoleMessage: (level, message, lineNumber, sourceID) => {
                if (message.startsWith("GAMEBUDDY_LAYOUT ")) buddy.save_widget_layout(message.slice(17))
                else if (message.startsWith("GAMEBUDDY_WIDGET_EVENTS ")) buddy.note_widget_events(message.slice(24))
                else if(message.startsWith("GAMEBUDDY_REGIONS "))root.widgetRegions=message.slice(18)
                else if(message.startsWith("GAMEBUDDY_DISMISS "))buddy.set_widget_visible(message.slice(18),false)
                else if(message.startsWith("GAMEBUDDY_READER "))buddy.guide_browser_request(message.slice(17))
                else if(message.startsWith("GAMEBUDDY_STEP "))buddy.select_step(message.slice(15))
                else if(message.startsWith("GAMEBUDDY_COMPLETE ")){buddy.select_step(message.slice(19));buddy.complete_step()}
                else if(message.startsWith("GAMEBUDDY_MAP_REQUEST ")){
                    buddy.map_preview(message.slice(22))
                }
                else if(message.startsWith("GAMEBUDDY_SOURCE_OPEN ")){
                    try { const url=JSON.parse(message.slice(22)); if(/^https?:\/\//.test(url)) Qt.openUrlExternally(url) } catch(e) {}
                }
                else if(root.appArguments.indexOf("--navigation-trace")>=0 && (message.startsWith("GAMEBUDDY_MAP_VIEW ") || message.startsWith("GAMEBUDDY_OUTLINE ") || message.startsWith("GAMEBUDDY_PICKER ") || message.startsWith("GAMEBUDDY_PROVENANCE ") || message.startsWith("GAMEBUDDY_WIDGET_SOURCES ")))console.warn(message)
            }
            onPermissionRequested: permission => permission.deny()
            onNavigationRequested: request => { if (request.url.toString() !== "about:blank" && !request.url.toString().startsWith("data:")) request.action = WebEngineView.IgnoreRequest }
        }
        Connections { target: root; function onSessionChanged() { widgetView.updateWidgets() } function onEditOverlayChanged() { widgetView.updateWidgets() } }
        Connections { target: buddy; function onGuide_browser_response(response) { widgetView.runJavaScript("window.gamebuddy && window.gamebuddy.readerResponse("+response+")") } }
    }
    Timer {
        interval: 32; repeat: true; running: compositor.active
        onTriggered: {
            root.overlayAction(compositor.poll(!root.overlayHidden,
                                companion.expanded || root.editOverlay || root.overlayOptions || root.cropOpen,
                                Qt.rect(companion.x, companion.y, companion.width, companion.height),
                                widgetView.visible ? root.widgetRegions : "[]"))
        }
    }
    Shortcut {
        sequence: "Escape"; context: Qt.ApplicationShortcut
        enabled: root.cropOpen || (compositor.active && (root.overlayOptions || companion.expanded || root.editOverlay))
        onActivated: root.releaseOverlay()
    }
    Text {
        parent: overlayScene
        visible: compositor.active && compositor.error.length > 0
        x: 24; y: parent.height-height-20; z: 30
        text: compositor.error; color: "#ffb4a8"; font.pixelSize: 14
    }
    CropEditor {
        id: cropEditor;parent:overlayScene;anchors.fill:parent;z:40
        visible:root.cropOpen;buddy:root.buddyObject
        trace:root.appArguments.indexOf("--crop-trace")>=0
        onClosed:root.cropOpen=false
    }
    Item {
        id: workbenchScene
        parent: compositor.active ? overlayScene : root.contentItem
        visible: !compositor.active || root.overlayOptions
        anchors.fill: parent
        z: 20
        Rectangle { anchors.fill: parent; color: "#0e1520" }
    }
    ColumnLayout {
        id: workbenchContent
        parent: workbenchScene
        visible: !compositor.active || root.overlayOptions
        z: 20
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
                GbButton { visible: compositor.active; text: "Back to game"; onClicked: { root.overlayOptions = false; companion.expanded = false; root.editOverlay = false } }
                GbButton { id: overlayButton; text: checked ? "Hide overlay" : "Show overlay"; checkable: true }
                GbButton { text: checked ? "Done arranging" : "Arrange widgets"; checkable: true; onToggled: { root.editOverlay = checked; if (checked) overlayButton.checked = true } }
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
                                    // Exactly what the AI sees: the newest cropped capture,
                                    // refreshed every second.
                                    Image {
                                        id: seenFrame
                                        property double capturedAt: 0
                                        property string jpeg: ""
                                        anchors.fill: parent; fillMode: Image.PreserveAspectFit; cache: false
                                        visible: !capture.active && !replay.source.toString().length
                                        source: jpeg.length ? "data:image/jpeg;base64," + jpeg : (root.session.frame_url || "")
                                        GeminiVoice { id: seenFrameSource }
                                        Timer {
                                            interval: 1000; repeat: true; triggeredOnStart: true
                                            running: seenFrame.visible && root.visible && !!root.session.data_dir
                                            onTriggered: {
                                                const serialized = seenFrameSource.latest_frame(root.session.data_dir, seenFrame.capturedAt)
                                                if (!serialized.length) return
                                                const frame = JSON.parse(serialized)
                                                seenFrame.capturedAt = frame.at; seenFrame.jpeg = frame.jpeg
                                            }
                                        }
                                    }
                                    Column {
                                        anchors.centerIn: parent; spacing: 14
                                        visible: !capture.active && !replay.source.toString().length && !root.session.frame_url
                                        Text { anchors.horizontalCenter: parent.horizontalCenter; text: compositor.active ? "Game connected" : "Connect your game"; color: root.ink; font.pixelSize: 21 }
                                        Text { anchors.horizontalCenter: parent.horizontalCenter; text: compositor.active ? (root.session.reference_count ? "Waiting for a game frame" : "Add a walkthrough to enable synchronization") : "Share a screen or compare a screenshot"; color: root.muted; font.pixelSize: 12 }
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
                            GbButton { visible: !compositor.active; text: capture.active ? "Stop sharing" : "Share screen"; onClicked: { replay.stop(); replay.source = ""; root.captureError = ""; capture.active = !capture.active } }
                            GbButton { text: "Replay video"; onClicked: replayDialog.open() }
                            GbButton { text: "Compare screenshot"; enabled: !buddy.busy && root.session.reference_count > 0; onClicked: frameDialog.open() }
                            GbButton { text: "Observe frame"; enabled: !buddy.matching && (compositor.active || capture.active || replay.source.toString().length > 0); onClicked: root.grabFrame() }
                            GbButton { text: "Crop game view"; enabled:!!root.session.frame_url;onClicked:root.openCrop() }
                            GbButton { text: checked ? "Auto sync on" : "Auto sync off"; checkable: true; checked: root.autoSync; onClicked: root.autoSync = checked }
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
                        Text { text: buddy.chat_error || buddy.answer || "Ask for guidance, add a goal, choose a reference, or revise the outline. Hold the companion microphone button to speak."; color: buddy.chat_error ? "#f6b7a5" : buddy.answer ? root.ink : root.muted; font.pixelSize: 13; wrapMode: Text.WordWrap; textFormat: Text.PlainText; Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24; Layout.bottomMargin: 24 }
                    }
                }
                // Walkthrough library
                ColumnLayout {
                    spacing: 14
                    Text { text: "Build your walkthrough"; color: root.ink; font.pixelSize: 26; Layout.leftMargin: 24; Layout.topMargin: 24 }
                    RowLayout {
                        Layout.leftMargin: 24; Layout.rightMargin: 24
                        GbButton { text: "YouTube video"; accent: true; enabled: !buddy.busy; onClicked: youtubeDialog.open() }
                        GbButton { text: "Find guide automatically"; enabled: !buddy.busy && !buddy.preparing; onClicked: buddy.prepare_game() }
                        GbButton { text: "Paste text"; enabled: !buddy.busy; onClicked: textDialog.open() }
                        GbButton { text: "Import file"; enabled: !buddy.busy; onClicked: fileDialog.open() }
                        ComboBox {
                            Layout.fillWidth: true; textRole: "title"; valueRole: "id"; model: root.session.guides || []
                            onActivated: buddy.select_guide(currentValue)
                        }
                    }
                    PreparationStatus {
                        objectName: "workbenchPreparation"
                        Layout.fillWidth: true; Layout.leftMargin: 24; Layout.rightMargin: 24
                        Layout.maximumHeight: 520; clip: true
                        report: JSON.parse(buddy.preparation_json)
                        onRetryRequested: buddy.prepare_game()
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
                                    GbButton { text: root.session.book_url ? "Open illustrated guide" : "Build illustrated guide"; enabled: !!root.session.guide && !buddy.busy; onClicked: { if(root.session.book_url)Qt.openUrlExternally(root.session.book_url);else buddy.export_illustrated() } }
                                    GbButton { text: "Compile substeps"; enabled: !!root.selected && !buddy.busy; onClicked: buddy.compile_selected() }
                                }
                                Rectangle { Layout.fillWidth: true; height: 1; color: "#2b3949" }
                                Text { text: "VISUAL REFERENCES"; color: "#86decb"; font.pixelSize: 11 }
                                Text { text: "Sample the walkthrough, then compare live gameplay. Coordinates below crop the reference video. Use Crop game view to select the play area in your own game capture."; color: root.muted; wrapMode: Text.WordWrap; Layout.fillWidth: true; font.pixelSize: 12 }
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
                ModelSettings { backend: buddy; audioStatus: gameAudio.status }
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
            const target = compositor.active ? overlayScene : root.visible ? workbenchScene : companion.snapshotItem
            console.warn("GAMEBUDDY_SMOKE target=" + target.width + "x" + target.height + " visible=" + target.visible + " companion=" + companionWindow.visible)
            const accepted = target.grabToImage(function(result) {
                console.warn("GAMEBUDDY_SMOKE saved=" + result.saveToFile(root.smokePath) + " guides=" + (root.session.guides || []).length)
                Qt.quit()
            })
            if (!accepted) { console.warn("GAMEBUDDY_SMOKE capture rejected"); Qt.exit(1) }
        }
    }
    Timer { interval: 8000; running: root.smokePath.length > 0; onTriggered: { console.warn("GAMEBUDDY_SMOKE timed out waiting for a rendered frame"); Qt.exit(1) } }
    Component.onCompleted: {
        if(smokePath.length && root.appArguments.indexOf("--smoke-models")>=0)root.page=2
        console.warn("GAMEBUDDY_READY game=" + (session.game || "") + " smoke=" + smokePath)
        if (replayPath.length > 0) replay.play()
    }
    Component.onDestruction: if (root.queuedVoice.length) buddy.discard_voice(root.queuedVoice)
}
