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
    property bool started: false
    property bool connected: false
    property bool capturing: false
    property bool failed: false
    property bool automaticSession: false
    property bool proactivePaused: false
    property bool localConversationActive: false
    property bool trace: false
    property string pageOrigin: ""
    property bool userSpeaking: false
    property bool detectedSpeech: false
    property bool assistantSpeaking: false
    property bool backendBusy: false
    property double inputAt: 0
    property double outputAt: 0
    property bool waitingForReply: false
    property bool returningPlayer: false
    property string voiceGame: ""
    property string pendingCue: ""
    property string captionText: ""
    property bool captionStreaming: false
    readonly property bool conversationActive: started && (userSpeaking || assistantSpeaking || waitingForReply || backendBusy)
    function clearActivity(){userSpeaking=false;detectedSpeech=false;assistantSpeaking=false;backendBusy=false;waitingForReply=false;pendingCue="";captionText="";captionStreaming=false;transport.reset_speech()}
    function assistantTranscript(text, complete) {
        // The transport can produce unsolicited speech. Captions follow the
        // same requested reply as playback, including when output is muted.
        if(!waitingForReply && !assistantSpeaking && !captionStreaming)return
        if(complete){captionText=text;captionStreaming=false}
        else {captionText=(captionStreaming?captionText:"")+text;captionStreaming=true}
        if(trace && captionText.length)console.warn("GAMEBUDDY_CAPTION_UPDATE characters="+captionText.length+" final="+complete)
        if(captionText.length)panel.answer(captionText)
    }
    function updateStatus() {
        if(!started || !connected || failed)return
        status=userSpeaking?"Listening…":assistantSpeaking?(capturing?"Speaking · you can interrupt":"Speaking · microphone off"):
            backendBusy?"Checking your request…":waitingForReply?"Waiting for voice…":
            capturing?"Live voice · speak naturally":"Guide voice ready · mic off"
    }
    onUserSpeakingChanged: updateStatus()
    onAssistantSpeakingChanged: updateStatus()
    onWaitingForReplyChanged: updateStatus()
    onBackendBusyChanged: updateStatus()
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
        return JSON.stringify({game:session.game, screen:session.screen, current:session.current, cursor:session.navigation?.cursor, location:session.navigation?.location, location_status:session.navigation?.location?.status,
            guidance:session.navigation?.guidance, spatial:session.navigation?.spatial, current_place:session.navigation?.map?.current_place, target_place:session.navigation?.map?.target_place,
            goals:session.guide?.goals || [], coverage_notes:(session.guide?.coverage_notes || []).slice(-6),
            preparing:buddy.preparing, guide_revision:session.guide?.revision, microphone_active:voice.active, returning_player:returningPlayer,
            conversation_active:conversationActive || localConversationActive || voice.speaking || buddy.busy,
            instruction:"This is automatic game-screen context, not a player request. GameBuddy continuously captures the game and the configured vision model describes changes in screen.scene. Observations are estimates with timestamps. get_game_screen returns the actual latest screenshot as IMAGE content to your reasoning tool; call it for screen questions or closer inspection before claiming you cannot see. A missing location match is not missing screen access. The application sends selected spoken event and next-action cues separately; stay quiet for routine context updates. Respect unconfirmed prerequisites. Room identity does not establish inventory, player coordinates, or completion. Never mark tasks complete from this update."})
    }
    function update() {
        if (!loaded) return
        if(started && voiceGame!==session.game){
            started=false;connected=false;transport.disconnect();runJavaScript("gamebuddyVoice.stop()")
        }
        const wanted=enabledBackend && !failed && !!session.game && (voice.active || (automaticSession && spokenReplies && !proactivePaused))
        if(wanted && !started) {
            voiceGame=session.game;returningPlayer=false
            started=true; status="Connecting live voice…"; lastContext=voiceContext()
            runJavaScript("gamebuddyVoice.muteOutput("+(!spokenReplies)+");gamebuddyVoice.start("+JSON.stringify(voice.device_name)+","+voice.active+")")
        } else if(!wanted && started) {
            started=false; connected=false; transport.disconnect(); runJavaScript("gamebuddyVoice.stop()")
            status="Voice stopped"
        }
    }
    function voiceContext() {
        const value=JSON.parse(context()),scene=value.screen?.scene
        // Keep the voice model's background context small and readable. Dense
        // geometry, map coordinates and diagnostics belong in its tools/OSD.
        return JSON.stringify({game:value.game,location_status:value.location_status,
            current_place:value.current_place?.label,
            screen:{status:value.screen?.status,captured_at_ms:value.screen?.captured_at_ms,
                scene_captured_at_ms:scene?.captured_at_ms,scene_fresh:scene?.fresh===true,
                summary:scene?.fresh?(scene.summary||'').slice(0,600):null},
            instruction:"Background reference only. Do not answer gameplay questions from cached advice or this summary. Delegate the complete current request and any location correction to the reasoning backend, then wait for its answer. Never speak the player's side of a conversation."})
    }
    function updateContext() {
        if(!connected)return
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
            if(trace)console.warn("GAMEBUDDY_NARRATION characters="+text.length)
            panel.answer(text)
            if(enabledBackend){
                // This cue already contains the selected words. Appending
                // commentary here can race and cancel its queued speech.
                // Authorize playback before the cue can produce remote audio.
                pendingCue=text
                runJavaScript("gamebuddyVoice.allowReply('cue')",()=>transport.speak(text))
            }
        }
    }
    Timer { interval: 1000; repeat: true; running: panel.automaticSession; onTriggered: panel.narrate() }
    // Keep continuous perception local. Refresh on player speech;
    // background appends can interrupt a queued voice response. Gameplay
    // questions also obtain a fresh screenshot through get_game_context.
    Timer {
        interval:100;repeat:true;running:panel.started
        onTriggered:{
            const now=Date.now()
            if(now-panel.inputAt>1000 && panel.detectedSpeech){panel.detectedSpeech=false;transport.reset_speech();runJavaScript("gamebuddyVoice.playerSpeech(false)")}
            if(now-panel.outputAt>1000)panel.assistantSpeaking=false
        }
    }
    RealtimeVoice {
        id: transport
        onEvent_jsonChanged: {
            const event=JSON.parse(event_json), p=event.params || {}
            if(panel.trace && event.method==="thread/realtime/transcript/done")console.warn("GAMEBUDDY_REALTIME_TRANSCRIPT role="+p.role+" characters="+(p.text||"").length)
            if(event.method==="gamebuddy/conversation")panel.returningPlayer=p.returning_player===true
            else if(event.method==="gamebuddy/reasoning"){
                panel.backendBusy=p.active===true
                panel.runJavaScript("gamebuddyVoice.setBackendBusy("+panel.backendBusy+")")
            }
            else if(event.method==="thread/realtime/sdp")panel.runJavaScript("gamebuddyVoice.answer("+JSON.stringify(p.sdp)+")")
            else if(event.method==="thread/realtime/error")panel.fail(p.message || "Voice connection failed")
            else if(event.method==="thread/realtime/closed")panel.fail("Voice session ended")
            else if(event.method==="thread/realtime/transcript/delta" && p.role==="assistant")panel.assistantTranscript(p.delta || "",false)
            else if(event.method==="thread/realtime/transcript/done") {
                if(p.role==="user")panel.transcript(p.text)
                else if(p.role==="assistant")panel.assistantTranscript(p.text || "",true)
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
        if(trace && !["input-pcm","output-active","offer"].includes(event.type))console.warn("GAMEBUDDY_VOICE_EVENT "+JSON.stringify(event))
        if(event.type==="capture"){capturing=event.value;if(!capturing){userSpeaking=false;detectedSpeech=false;transport.reset_speech();runJavaScript("gamebuddyVoice.playerSpeech(false)")};return}
        if(!started)return
        if(event.type==="input-pcm"){
            if(!capturing)return
            inputAt=Date.now()
            const speaking=transport.detect_speech(JSON.stringify(event.value))
            if(speaking!==detectedSpeech){
                runJavaScript("gamebuddyVoice.playerSpeech("+speaking+")")
            }
            detectedSpeech=speaking
        }
        else if(event.type==="input-active"){
            userSpeaking=event.value
            if(userSpeaking){pendingCue="";if(!captionStreaming)captionText=""}
            transport.player_speech(userSpeaking)
            if(userSpeaking)updateContext()
        }
        else if(event.type==="output-active"){
            if(event.value)pendingCue=""
            if(trace && event.value && !assistantSpeaking)console.warn("GAMEBUDDY_VOICE_PLAYBACK started")
            outputAt=Date.now();assistantSpeaking=event.value
        }
        else if(event.type==="reply-pending")waitingForReply=event.value
        else if(event.type==="cue-retry" && pendingCue.length)transport.speak(pendingCue)
        else if(event.type==="cue-timeout"){pendingCue="";status="Voice cue unavailable · you can still speak"}
        else if(event.type==="playback-allowed" || event.type==="reply-permit"){if(trace)console.warn("GAMEBUDDY_VOICE_GATE "+event.type+" "+event.value)}
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
    onCapturingChanged: updateStatus()
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
