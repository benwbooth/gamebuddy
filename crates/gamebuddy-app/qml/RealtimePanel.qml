import QtQuick
import QtWebEngine
import GameBuddy

WebEngineView {
    id: panel
    required property var buddy
    required property var voice
    required property var session
    property bool enabledBackend: false
    property bool spokenReplies: true
    property bool loaded: false
    property string status: ""
    property string lastContext: ""
    property double contextAfter: 0
    property bool started: false
    property bool connected: false
    property bool capturing: false
    property bool failed: false
    property bool automaticSession: false
    property bool proactivePaused: false
    property bool trace: false
    property string pageOrigin: ""
    property bool userSpeaking: false
    property bool assistantSpeaking: false
    property double replyUntil: 0
    property double inputAt: 0
    property double outputAt: 0
    property bool waitingForReply: false
    readonly property bool conversationActive: started && (userSpeaking || assistantSpeaking || waitingForReply)
    function clearActivity(){userSpeaking=false;assistantSpeaking=false;replyUntil=0;waitingForReply=false;transport.reset_speech()}
    onStartedChanged: if(!started)clearActivity()
    onStatusChanged: if(trace)console.warn("GAMEBUDDY_REALTIME_STATUS " + status)
    signal transcript(string text)
    signal answer(string text)
    // Keep the audio document attached to the live overlay window even when
    // the separate workbench is closed.
    width: 1; height: 1; visible: true; opacity: 0.01
    lifecycleState: WebEngineView.Active
    settings.playbackRequiresUserGesture: false
    settings.localContentCanAccessFileUrls: false
    settings.javascriptCanOpenWindows: false
    function context() {
        return JSON.stringify({game:session.game, screen:session.screen, current:session.current, location_status:session.navigation?.location?.status,
            guidance:session.navigation?.guidance, current_place:session.navigation?.map?.current_place, target_place:session.navigation?.map?.target_place,
            goals:session.guide?.goals || [], coverage_notes:(session.guide?.coverage_notes || []).slice(-6),
            preparing:buddy.preparing, guide_revision:session.guide?.revision,
            instruction:"This is automatic game-screen context, not a player request. GameBuddy continuously captures the game and the configured vision model describes changes in screen.scene. Observations are estimates with timestamps. get_game_screen returns the actual latest screenshot as IMAGE content to your reasoning tool; call it for screen questions or closer inspection before claiming you cannot see. A missing location match is not missing screen access. The application sends selected spoken event and next-action cues separately; stay quiet for routine context updates. Respect unconfirmed prerequisites. Room identity does not establish inventory, player coordinates, or completion. Never mark tasks complete from this update."})
    }
    function update() {
        if (!loaded) return
        const wanted=enabledBackend && !failed && (voice.active || (automaticSession && spokenReplies && !proactivePaused && !!session.game))
        if(wanted && !started) {
            started=true; status="Connecting live voice…"; lastContext=context()
            runJavaScript("gamebuddyVoice.muteOutput("+(!spokenReplies)+");gamebuddyVoice.start("+JSON.stringify(voice.device_name)+","+voice.active+")")
        } else if(!wanted && started) {
            started=false; connected=false; transport.disconnect(); runJavaScript("gamebuddyVoice.stop()")
            status="Voice stopped"
        }
    }
    function voiceContext() {
        const value=JSON.parse(context()),screen=value.screen
        // Frame age is available from tools. Do not stream a new context message
        // on every UI refresh when the observation itself has not changed.
        if(screen){delete screen.age_ms;delete screen.captured_at_ms
            if(screen.scene){delete screen.scene.age_ms;delete screen.scene.captured_at_ms}}
        return JSON.stringify(value)
    }
    function updateContext() {
        if(!connected || Date.now()<contextAfter)return
        const next=voiceContext()
        if(next!==lastContext){lastContext=next;runJavaScript("gamebuddyVoice.updateContext("+JSON.stringify(next)+")")}
    }
    function fail(message) {
        failed=true
        if(voice.active)voice.toggle_listening()
        update(); status=message
    }
    function stopSpeaking() {
        proactivePaused=true
        if(enabledBackend && voice.active)voice.toggle_listening()
        if(started){started=false;connected=false;transport.disconnect();runJavaScript("gamebuddyVoice.stop()")}
        status="Voice paused · choose Listen or turn Voice on to resume"
    }
    function narrate() {
        if(!spokenReplies || proactivePaused || (enabledBackend && !connected))return
        const text=transport.next_narration(context())
        if(text.length){
            // Do not race a quiet context burst against a queued spoken cue.
            // Capture and visual inference continue; send the newest context afterward.
            contextAfter=Date.now()+12000
            if(trace)console.warn("GAMEBUDDY_NARRATION characters="+text.length)
            panel.answer(text)
            if(enabledBackend)transport.speak(text)
        }
    }
    Timer { interval: 1000; repeat: true; running: panel.automaticSession; onTriggered: panel.narrate() }
    Timer { interval: 2000; repeat: true; running: panel.connected; onTriggered: panel.updateContext() }
    Timer {
        interval:100;repeat:true;running:panel.started
        onTriggered:{
            const now=Date.now()
            if(now-panel.inputAt>1000 && panel.userSpeaking){panel.userSpeaking=false;transport.reset_speech()}
            if(now-panel.outputAt>1000)panel.assistantSpeaking=false
            panel.waitingForReply=panel.replyUntil>now
        }
    }
    RealtimeVoice {
        id: transport
        onEvent_jsonChanged: {
            const event=JSON.parse(event_json), p=event.params || {}
            if(panel.trace && event.method==="thread/realtime/transcript/done")console.warn("GAMEBUDDY_REALTIME_TRANSCRIPT role="+p.role+" characters="+(p.text||"").length)
            if(event.method==="thread/realtime/sdp")panel.runJavaScript("gamebuddyVoice.answer("+JSON.stringify(p.sdp)+")")
            else if(event.method==="thread/realtime/error")panel.fail(p.message || "Voice connection failed")
            else if(event.method==="thread/realtime/closed")panel.fail("Voice session ended")
            else if(event.method==="thread/realtime/transcript/done") {
                if(p.role==="user")panel.transcript(p.text)
                else if(p.role==="assistant")panel.answer(p.text)
            }
        }
    }
    Component.onCompleted: {
        const page=transport.page_url()
        pageOrigin=page.slice(0,page.lastIndexOf('/')+1)
        url=page
    }
    Component.onDestruction: transport.disconnect()
    onLoadingChanged: request => { if(request.status===WebEngineView.LoadSucceededStatus){loaded=true;update()} }
    onPermissionRequested: permission => {
        if(trace)console.warn("GAMEBUDDY_VOICE_PERMISSION type="+permission.permissionType+" origin="+permission.origin+" expected="+pageOrigin+" state="+permission.state)
        if(enabledBackend && voice.active && permission.origin.toString()===pageOrigin
                && permission.permissionType===WebEnginePermission.PermissionType.MediaAudioCapture) permission.grant()
        else permission.deny()
    }
    onNavigationRequested: request => {
        if(!pageOrigin.length || request.url.toString()!==pageOrigin+"voice")request.action=WebEngineView.IgnoreRequest
    }
    onJavaScriptConsoleMessage: (level,message,line,source) => {
        if(!message.startsWith("GAMEBUDDY_VOICE "))return
        const event=JSON.parse(message.slice(16))
        if(event.type==="capture"){capturing=event.value;if(!capturing){userSpeaking=false;transport.reset_speech()};return}
        if(!started)return
        if(event.type==="input-pcm"){
            if(!capturing)return
            inputAt=Date.now()
            const speaking=transport.detect_speech(JSON.stringify(event.value))
            if(speaking && !userSpeaking){replyUntil=Date.now()+20000;waitingForReply=true}
            if(!speaking && userSpeaking && waitingForReply)replyUntil=Date.now()+20000
            userSpeaking=speaking
        }
        else if(event.type==="output-active"){
            outputAt=Date.now();assistantSpeaking=event.value
            if(assistantSpeaking){replyUntil=0;waitingForReply=false}
        }
        else if(event.type==="offer")transport.connect_offer(event.value,session.data_dir,panel.lastContext)
        else if(event.type==="error")fail(event.value)
        else if(event.type==="microphone-error") { if(voice.active)voice.toggle_listening();status="Microphone: "+event.value }
        else if(event.type==="ready") { connected=true;status=capturing?"Live voice · speak naturally":"Guide voice ready · mic off";narrate() }
        else if(event.type==="connection") {
            if(event.value==="failed" || event.value==="disconnected")fail("Voice connection lost; choose Listen to reconnect")
        }
    }
    onEnabledBackendChanged: update()
    onSpokenRepliesChanged: { if(spokenReplies){failed=false;proactivePaused=false};if(loaded)runJavaScript("gamebuddyVoice.muteOutput("+(!spokenReplies)+")");update() }
    onCapturingChanged: if(connected)status=capturing?"Live voice · speak naturally":"Guide voice ready · mic off"
    Connections { target: panel.voice; function onActiveChanged(){
        if(panel.voice.active){panel.failed=false;panel.proactivePaused=false}
        const wasStarted=panel.started
        panel.update()
        if(wasStarted && panel.started)panel.runJavaScript("gamebuddyVoice.setMicrophone("+panel.voice.active+","+JSON.stringify(panel.voice.device_name)+")")
    } }
    onSessionChanged: {
        update()
    }
}
