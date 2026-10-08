import QtQuick
import QtWebEngine
import GameBuddy

// Gemini Live speech-to-speech. Gemini listens and speaks; every game
// question is answered by the configured reasoning provider (buddy.ask)
// and returned to Gemini as an ask_gamebuddy tool result.
WebEngineView {
    id: panel
    required property var buddy
    required property var voice
    required property var session
    property var contextSource: null
    property bool enabledBackend: false
    property bool spokenReplies: true
    property bool automaticSession: false
    property bool trace: false
    property bool loaded: false
    property bool started: false
    property bool connected: false
    property bool failed: false
    property bool proactivePaused: false
    property bool capturing: false
    property bool assistantSpeaking: false
    property bool workPending: false
    property double inputAt: 0
    property string status: ""
    property string pageOrigin: ""
    property string voiceGame: ""
    property string captionText: ""
    property bool captionStreaming: false
    property string pendingTool: ""
    property var queuedTools: []
    property string lastContextKey: ""
    property double lastContextAt: 0
    property string activeAdviceStep: ""
    property bool userSpeaking: false
    property string ownAnswer: ""
    readonly property bool conversationActive: started && (assistantSpeaking || workPending || userSpeaking)
    signal transcript(string text)
    signal answer(string text)
    width: 1; height: 1; visible: true; opacity: 0.01
    lifecycleState: WebEngineView.Active
    settings.playbackRequiresUserGesture: false
    settings.localContentCanAccessFileUrls: false
    settings.javascriptCanOpenWindows: false

    function context() {
        if (!contextSource) return JSON.stringify({game: session.game})
        const value = JSON.parse(contextSource.context())
        value.conversation_active = conversationActive || buddy.busy
        value.player_speaking = userSpeaking
        return JSON.stringify(value)
    }
    // The live map match agrees with the current step's place, or the player
    // reported where they are. Otherwise the saved step may be for another
    // area, so it is neither repeated nor announced without Claude checking.
    function placeAgrees() {
        const map = session.navigation?.map
        return session.navigation?.location?.status === "reported" || !map?.current_place
            || !map?.target_place || map.current_place.id === map.target_place.id
    }
    function briefContext() {
        const scene = session.screen?.scene
        const guidance = session.navigation?.guidance
        // next_action is the step GameBuddy's assistant last published; Gemini
        // may repeat it for a plain "what's next?" only while it is ready.
        return JSON.stringify({game: session.game, current_task: session.current?.title, next_action: guidance?.now?.instructions,
            // Only when the map match agrees with the step's place (or the
            // player reported it); otherwise Claude must look at the screen.
            next_action_ready: guidance?.status === "ready" && !!guidance?.now?.instructions && placeAgrees(),
            place: session.navigation?.map?.current_place?.label, scene: scene?.fresh ? (scene.summary || "").slice(0, 400) : null})
    }
    function updateStatus() {
        if (!started || failed) return
        status = !connected ? "Connecting Gemini voice…" : assistantSpeaking ? "Speaking · you can interrupt"
            : workPending ? "Thinking…" : capturing ? "Live voice · speak naturally" : "Guide voice ready · mic off"
    }
    onAssistantSpeakingChanged: updateStatus()
    onWorkPendingChanged: updateStatus()
    onCapturingChanged: updateStatus()
    onConnectedChanged: updateStatus()
    onStatusChanged: if (trace) console.warn("GAMEBUDDY_GEMINI_STATUS " + status)
    function update() {
        if (!loaded) return
        if (started && voiceGame !== session.game) shutdown("Voice stopped")
        const wanted = enabledBackend && !failed && !!session.game && (voice.active || (automaticSession && spokenReplies && !proactivePaused))
        if (wanted && !started) {
            started = true; connected = false; voiceGame = session.game; updateStatus()
            const setup = gemini.session(session.data_dir || "", session.game, briefContext())
            runJavaScript("gamebuddyGemini.muteOutput(" + (!spokenReplies) + ");gamebuddyGemini.start(" + setup + "," + JSON.stringify(voice.device_name) + "," + voice.active + ")")
        } else if (!wanted && started) shutdown("Voice stopped")
    }
    function shutdown(message) {
        started = false; connected = false; assistantSpeaking = false; workPending = false; captionStreaming = false
        if (pendingTool.length) pendingTool = ""
        queuedTools = []
        runJavaScript("gamebuddyGemini.stop()")
        status = message
    }
    function fail(message) {
        failed = true
        if (voice.active) voice.toggle_listening()
        shutdown(message)
    }
    function stopSpeaking() {
        proactivePaused = true
        if (enabledBackend && voice.active) voice.toggle_listening()
        if (started) shutdown("Voice paused · choose Listen or turn Voice on to resume")
    }
    function finishTool(text, failedRequest) {
        if (!pendingTool.length) return
        const id = pendingTool; pendingTool = ""; workPending = false
        runJavaScript("gamebuddyGemini.toolResult(" + JSON.stringify(id) + "," + JSON.stringify(text) + "," + failedRequest + ")")
        Qt.callLater(startQueuedTool)
    }
    // Requests made while GameBuddy is busy wait their turn. Refusing them
    // made Gemini improvise answers (prices, items) instead of waiting.
    function startQueuedTool() {
        if (pendingTool.length || !queuedTools.length) return
        if (buddy.busy) { Qt.callLater(startQueuedTool); return }
        const next = queuedTools[0]
        queuedTools = queuedTools.slice(1)
        if (buddy.ask(next.request)) { pendingTool = next.id; workPending = true }
        else runJavaScript("gamebuddyGemini.toolResult(" + JSON.stringify(next.id) + "," + JSON.stringify("GameBuddy could not start that request. Ask the player to repeat it.") + ",true)")
    }
    function narrate() {
        if (!connected || !spokenReplies || proactivePaused || assistantSpeaking || workPending || userSpeaking || buddy.busy || !placeAgrees()) return
        const serialized = transport.next_narration(context())
        if (!serialized.length) return
        const cue = JSON.parse(serialized)
        if (trace) console.warn("GAMEBUDDY_NARRATION characters=" + cue.text.length)
        if (cue.urgent) runJavaScript("gamebuddyGemini.interrupt()")
        activeAdviceStep = cue.urgent ? "" : (session.navigation?.guidance?.now?.id || "")
        ownAnswer = cue.text; panel.answer(cue.text)
        runJavaScript("gamebuddyGemini.cue(" + JSON.stringify(cue.text) + ")")
    }
    // Keep Gemini's background picture current without making it speak.
    function shareContext() {
        if (!connected || assistantSpeaking || userSpeaking) return
        const key = (session.navigation?.guidance?.now?.id || "") + "|" + (session.navigation?.guidance?.status || "") + "|" + (session.navigation?.map?.current_place?.label || "")
        if (key === lastContextKey || Date.now() - lastContextAt < 3000) return
        lastContextKey = key; lastContextAt = Date.now()
        runJavaScript("gamebuddyGemini.context(" + JSON.stringify(briefContext()) + ")")
    }
    Timer { interval: 250; repeat: true; running: panel.started && panel.automaticSession; onTriggered: panel.narrate() }
    Timer { interval: 1000; repeat: true; running: panel.started; onTriggered: panel.shareContext() }
    // Never leave the player waiting on a request that stalled.
    Timer {
        interval: 90000; running: panel.pendingTool.length > 0
        onTriggered: panel.finishTool("GameBuddy took too long to answer. Ask the player to try again.", true)
    }
    Timer { interval: 250; repeat: true; running: panel.userSpeaking; onTriggered: if (Date.now() - panel.inputAt > 1500) panel.userSpeaking = false }
    GeminiVoice { id: gemini }
    // Only its narrator is used; the Codex transport never connects here.
    RealtimeVoice { id: transport }
    Connections {
        target: panel.buddy
        function onAnswerChanged() { if (panel.pendingTool.length && panel.buddy.answer.length && panel.buddy.answer !== panel.ownAnswer) panel.finishTool(panel.buddy.answer, false) }
        function onChat_errorChanged() { if (panel.pendingTool.length && panel.buddy.chat_error.length) panel.finishTool(panel.buddy.chat_error, true) }
        function onBusyChanged() { if (!panel.buddy.busy && panel.pendingTool.length) Qt.callLater(function() { panel.finishTool("GameBuddy could not answer that request.", true) }) }
    }
    Component.onCompleted: {
        const page = gemini.page_url()
        pageOrigin = page.slice(0, page.lastIndexOf('/') + 1)
        url = page
    }
    onLoadingChanged: request => { if (request.status === WebEngineView.LoadSucceededStatus) { loaded = true; update() } }
    onPermissionRequested: permission => {
        if (enabledBackend && voice.active && permission.origin.toString() === pageOrigin
                && permission.permissionType === WebEnginePermission.PermissionType.MediaAudioCapture) permission.grant()
        else permission.deny()
    }
    onNavigationRequested: request => { if (!pageOrigin.length || request.url.toString() !== pageOrigin + "gemini") request.action = WebEngineView.IgnoreRequest }
    onJavaScriptConsoleMessage: (level, message, line, source) => {
        if (!message.startsWith("GAMEBUDDY_GEMINI ")) return
        const event = JSON.parse(message.slice(17)), v = event.value
        if (trace && event.type !== "input-text" && event.type !== "caption") console.warn("GAMEBUDDY_GEMINI_EVENT " + JSON.stringify(event))
        if (!started) return
        if (event.type === "ready") { connected = true; updateStatus(); if (automaticSession) narrate() }
        else if (event.type === "reconnecting") { connected = false; status = "Reconnecting Gemini voice…" }
        else if (event.type === "error") fail("Gemini voice: " + v)
        else if (event.type === "microphone-error") { if (voice.active) voice.toggle_listening(); status = "Microphone: " + v }
        else if (event.type === "capture") capturing = v
        else if (event.type === "input-text") { inputAt = Date.now(); userSpeaking = true }
        else if (event.type === "input-final") { userSpeaking = false; gemini.record(session.data_dir || "", session.game, "user", v); panel.transcript(v) }
        else if (event.type === "caption") { captionText = v; captionStreaming = true }
        else if (event.type === "output-final") {
            captionStreaming = false
            gemini.record(session.data_dir || "", session.game, "assistant", v)
            // A filler spoken while the reasoning provider works is not its answer.
            if (!pendingTool.length) { ownAnswer = v; panel.answer(v) }
        }
        else if (event.type === "output-active") {
            if (v && !assistantSpeaking && activeAdviceStep.length && activeAdviceStep === session.navigation?.guidance?.now?.id) transport.advice_spoken(context())
            assistantSpeaking = v
            if (!v) captionStreaming = false
        }
        else if (event.type === "interrupted") { captionStreaming = false; activeAdviceStep = "" }
        else if (event.type === "tool-call") {
            queuedTools = queuedTools.concat([{id: v.id, request: v.request}])
            workPending = true
            startQueuedTool()
        }
        else if (event.type === "tool-cancel") {
            queuedTools = queuedTools.filter(t => t.id !== v)
            if (pendingTool === v) { pendingTool = ""; workPending = queuedTools.length > 0; Qt.callLater(startQueuedTool) }
        }
    }
    onEnabledBackendChanged: update()
    onSpokenRepliesChanged: { if (spokenReplies) { failed = false; proactivePaused = false }; if (loaded) runJavaScript("gamebuddyGemini.muteOutput(" + (!spokenReplies) + ")"); update() }
    onSessionChanged: update()
    Connections { target: panel.voice; function onActiveChanged() {
        if (panel.voice.active) { panel.failed = false; panel.proactivePaused = false }
        const wasStarted = panel.started
        panel.update()
        if (wasStarted && panel.started) panel.runJavaScript("gamebuddyGemini.setMicrophone(" + panel.voice.active + "," + JSON.stringify(panel.voice.device_name) + ")")
    } }
    Component.onDestruction: runJavaScript("gamebuddyGemini.stop()")
}
