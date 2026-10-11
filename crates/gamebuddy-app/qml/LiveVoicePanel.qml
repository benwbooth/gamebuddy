import QtQuick
import QtWebEngine
import GameBuddy

// Gemini Live speech-to-speech. Gemini listens, speaks and reasons: it sees
// the game through frames streamed about once a second and calls
// GameBuddy's tools (route, location, next action and arrow, journal).
WebEngineView {
    id: panel
    required property var buddy
    required property var voice
    required property var session
    property var contextSource: null
    property bool enabledBackend: false
    property bool spokenReplies: true
    property bool automaticSession: false
    // Unprompted remarks after scene changes (experimental, off by default).
    property bool commentary: false
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
    property int runningTools: 0
    property double lastFrameAt: 0
    property string lastContextKey: ""
    property double lastContextAt: 0
    property string activeAdviceStep: ""
    property bool userSpeaking: false
    property bool greeted: false
    property string ownAnswer: ""
    readonly property bool conversationActive: started && (assistantSpeaking || workPending || userSpeaking)
    signal transcript(string text)
    signal outOfTokens(string message)
    // The live service failed; the app falls back to local voice and retries.
    signal unavailable(string message)
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
    // area, so it is not announced unprompted.
    function placeAgrees() {
        const map = session.navigation?.map
        return session.navigation?.location?.status === "reported" || !map?.current_place
            || !map?.target_place || map.current_place.id === map.target_place.id
    }
    function briefContext() {
        const scene = session.screen?.scene
        const guidance = session.navigation?.guidance
        return JSON.stringify({game: session.game, current_task: session.current?.title, saved_next_action: guidance?.now?.instructions,
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
            started = true; connected = false; greeted = false; voiceGame = session.game; updateStatus()
            const setup = gemini.session(session.data_dir || "", session.game, briefContext())
            runJavaScript("gamebuddyGemini.muteOutput(" + (!spokenReplies) + ");gamebuddyGemini.start(" + setup + "," + JSON.stringify(voice.device_name) + "," + voice.active + ")")
        } else if (!wanted && started) shutdown("Voice stopped")
    }
    function shutdown(message) {
        started = false; connected = false; assistantSpeaking = false; workPending = false; captionStreaming = false
        runningTools = 0; lastFrameAt = 0
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
    function runTool(call) {
        runningTools++; workPending = true
        gemini.call_tool(session.data_dir || "", call.id, call.name, JSON.stringify(call.args || {}))
    }
    function attachStill() {
        const serialized = gemini.latest_frame(session.data_dir || "", 0)
        if (serialized.length) runJavaScript("gamebuddyGemini.still(" + JSON.stringify(JSON.parse(serialized).jpeg) + ")")
    }
    // The newest captured frame, so Gemini sees what the player sees now.
    function sendFrame() {
        if (!connected) return
        const serialized = gemini.latest_frame(session.data_dir || "", lastFrameAt)
        if (!serialized.length) return
        const frame = JSON.parse(serialized)
        lastFrameAt = frame.at
        runJavaScript("gamebuddyGemini.frame(" + JSON.stringify(frame.jpeg) + ")")
    }
    function narrate() {
        if (!connected || !spokenReplies || proactivePaused || assistantSpeaking || workPending || userSpeaking || !placeAgrees()) return
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
    // Gemini watches the feed and speaks up itself (watch checks below);
    // route-based narration cues could describe a stale position.
    Timer { interval: 1000; repeat: true; running: panel.started; onTriggered: panel.shareContext() }
    Timer { interval: 1000; repeat: true; running: panel.connected; onTriggered: panel.sendFrame() }
    // Like a friend watching along: after the scene changes, let Gemini say
    // something if it matters. It stays silent otherwise.
    property double lastWatchAt: 0
    Timer {
        interval: 2000; repeat: true; running: panel.commentary && panel.connected && panel.spokenReplies && !panel.proactivePaused
        onTriggered: {
            if (panel.assistantSpeaking || panel.userSpeaking || panel.workPending || Date.now() - panel.lastWatchAt < 6000) return
            if (!gemini.scene_changed(panel.session.data_dir || "")) return
            panel.lastWatchAt = Date.now()
            panel.runJavaScript("gamebuddyGemini.watch()")
        }
    }
    // Gemini sometimes goes silent after a tool result; prompt it to answer
    // rather than leave the player waiting.
    property int nudges: 0
    Timer {
        id: quietWatch
        interval: 9000
        onTriggered: {
            if (!panel.connected || panel.bookkeeping || panel.assistantSpeaking || panel.userSpeaking || panel.runningTools > 0 || panel.nudges >= 2) return
            panel.nudges++
            gemini.note(panel.session.data_dir || "", "nudge", JSON.stringify(panel.nudges))
            panel.runJavaScript("gamebuddyGemini.cue(" + JSON.stringify("You have the tool results. Answer the player now, in a sentence or two.") + ")")
            restart()
        }
    }
    // Silent bookkeeping: after a scene change (or every 30 s) Gemini
    // records items bought, found or used up, so the plan knows them.
    property double lastBookkeepAt: 0
    property bool bookkeeping: false
    Timer {
        interval: 3000; repeat: true; running: panel.connected
        onTriggered: {
            if (panel.assistantSpeaking || panel.userSpeaking || panel.workPending || Date.now() - panel.lastBookkeepAt < 8000) return
            const changed = gemini.scene_changed(panel.session.data_dir || "")
            if (!changed && Date.now() - panel.lastBookkeepAt < 30000) return
            panel.lastBookkeepAt = Date.now()
            panel.bookkeeping = true
            panel.runJavaScript("gamebuddyGemini.bookkeep()")
        }
    }
    Timer { interval: 250; repeat: true; running: panel.userSpeaking; onTriggered: if (Date.now() - panel.inputAt > 1500) panel.userSpeaking = false }
    GeminiVoice {
        id: gemini
        onTool_event_jsonChanged: {
            const event = JSON.parse(tool_event_json)
            if (panel.trace) console.warn("GAMEBUDDY_GEMINI_TOOL name=" + event.name + " ms=" + event.elapsed_ms + " error=" + !!event.response.error)
            gemini.note(panel.session.data_dir || "", "tool-result", JSON.stringify({name: event.name, ms: event.elapsed_ms, error: event.response.error || null}))
            panel.runningTools = Math.max(0, panel.runningTools - 1)
            panel.workPending = panel.runningTools > 0
            if (!panel.started) return
            // Tool results carry no images; attach the current screen itself.
            if (["get_game_screen", "get_game_context"].includes(event.name)) panel.attachStill()
            panel.runJavaScript("gamebuddyGemini.toolResult(" + JSON.stringify(event.id) + "," + JSON.stringify(event.response) + ")")
            if (panel.runningTools === 0) quietWatch.restart()
            // Published actions update the Now card, outline and arrow.
            panel.buddy.poll_widgets()
        }
    }
    // Only its narrator is used; the Codex transport never connects here.
    RealtimeVoice { id: transport }
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
        if (["ready", "frames", "tool-call", "error", "quota", "reconnecting", "input-final", "output-final", "capture", "microphone-error", "interrupted"].includes(event.type))
            gemini.note(session.data_dir || "", event.type, JSON.stringify(event.type === "tool-call" ? {name: v.name, args: v.args}
                // How old the newest frame Gemini had was when the player spoke.
                : event.type === "input-final" ? {text: v, frame_age_ms: lastFrameAt ? Date.now() - lastFrameAt : null} : v))
        if (event.type === "ready") {
            connected = true; updateStatus(); sendFrame()
            // Say hello once per session, not after reconnects.
            if (!greeted && spokenReplies && !proactivePaused) {
                greeted = true
                runJavaScript("gamebuddyGemini.cue(" + JSON.stringify("The player just started voice chat. Greet them in one short, casual line, then call get_game_context and tell them in a sentence where they are and what's next, or ask where they are if plan.area_confirmed is false.") + ")")
            }
        }
        else if (event.type === "reconnecting") { connected = false; status = "Reconnecting Gemini voice…" }
        else if (event.type === "error") { fail("Gemini voice: " + v); panel.unavailable(v || "Gemini Live closed the session.") }
        else if (event.type === "quota") { fail("Gemini voice is out of tokens"); panel.outOfTokens("Out of tokens: Gemini has no tokens or quota left. " + (v || "")); panel.unavailable("") }
        else if (event.type === "microphone-error") { if (voice.active) voice.toggle_listening(); status = "Microphone: " + v }
        else if (event.type === "capture") capturing = v
        else if (event.type === "input-text") {
            // A fresh full-quality still with every question, so answers
            // about the screen use what is there now.
            if (!userSpeaking) attachStill()
            inputAt = Date.now(); userSpeaking = true; bookkeeping = false; quietWatch.stop(); nudges = 0
        }
        else if (event.type === "input-final") {
            userSpeaking = false; gemini.record(session.data_dir || "", session.game, "user", v); panel.transcript(v)
            const area = gemini.player_said(session.data_dir || "", v)
            if (area.length) {
                gemini.note(session.data_dir || "", "spoken-location", JSON.stringify(area))
                runJavaScript("gamebuddyGemini.context(" + JSON.stringify("The player said they are in " + area + "; GameBuddy moved the walkthrough position there. Call get_game_context for the updated next step.") + ")")
            }
        }
        else if (event.type === "caption") { captionText = v; captionStreaming = true; quietWatch.stop(); nudges = 0 }
        else if (event.type === "output-final") {
            captionStreaming = false
            gemini.record(session.data_dir || "", session.game, "assistant", v)
            ownAnswer = v; panel.answer(v)
        }
        else if (event.type === "output-active") {
            if (v && !assistantSpeaking && activeAdviceStep.length && activeAdviceStep === session.navigation?.guidance?.now?.id) transport.advice_spoken(context())
            assistantSpeaking = v
            if (!v) captionStreaming = false
        }
        else if (event.type === "bookkeeping-done") bookkeeping = false
        else if (event.type === "interrupted") { captionStreaming = false; activeAdviceStep = "" }
        else if (event.type === "tool-call") { quietWatch.stop(); runTool(v) }
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
