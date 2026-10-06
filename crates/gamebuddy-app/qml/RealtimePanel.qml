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
    property bool conversational: false
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
    property bool workPending: false
    property double workStartedAt: 0
    property bool returningPlayer: false
    property string voiceGame: ""
    property string pendingCue: ""
    property string activeAdviceStep: ""
    property string lastNarrationState: ""
    property string recoveryCue: ""
    property int recoveryAttempts: 0
    property bool reconnecting: false
    property string captionText: ""
    property bool captionStreaming: false
    readonly property bool conversationActive: started && (userSpeaking || assistantSpeaking || waitingForReply || backendBusy)
    function clearActivity(){userSpeaking=false;detectedSpeech=false;assistantSpeaking=false;backendBusy=false;workPending=false;waitingForReply=false;pendingCue="";captionText="";captionStreaming=false;transport.reset_speech()}
    function speakApproved(text, urgent=false, visual=false, stepId="", delegationId=null) {
        if(!text.length || !connected)return
        // Show the completed answer immediately in the companion. Captions
        // have their own playback clock and must still wait for audible words.
        panel.answer(text)
        pendingCue=text
        activeAdviceStep=stepId
        if(urgent){transport.interrupt_reasoning();transport.cancel_speech();runJavaScript("gamebuddyVoice.interruptOutput()") }
        if(transport.speak_prepared(text,visual||urgent))return
        runJavaScript("gamebuddyVoice.queueSpeech("+JSON.stringify(text)+","+JSON.stringify(delegationId)+")")
    }
    function recoverSpeech(text) {
        if(recoveryAttempts>=1 || reconnecting || proactivePaused || !spokenReplies || !text.length)return false
        recoveryCue=text;recoveryAttempts++;reconnecting=true
        connected=false;started=false;transport.disconnect()
        status="Reconnecting voice · answer shown as text"
        runJavaScript("gamebuddyVoice.stop()", function(){reconnecting=false;panel.update()})
        return true
    }
    function updateStatus() {
        if(!started || !connected || failed)return
        status=userSpeaking?"Listening…":assistantSpeaking?(capturing?"Speaking · you can interrupt":"Speaking · microphone off"):
            workPending || backendBusy?"Thinking…"+(workStartedAt ? " · "+Math.floor((Date.now()-workStartedAt)/1000)+" s" : ""):waitingForReply?"Preparing voice…":
            capturing?"Live voice · speak naturally":"Guide voice ready · mic off"
    }
    onUserSpeakingChanged: {if(userSpeaking)activeAdviceStep="";updateStatus()}
    onAssistantSpeakingChanged: updateStatus()
    onWaitingForReplyChanged: updateStatus()
    onBackendBusyChanged: updateStatus()
    onWorkPendingChanged: { workStartedAt=workPending?Date.now():0;updateStatus() }
    Timer { interval: 250; repeat: true; running: panel.workPending; onTriggered: panel.updateStatus() }
    onStartedChanged: if(!started)clearActivity()
    onStatusChanged: if(trace)console.warn("GAMEBUDDY_REALTIME_STATUS " + status)
    signal transcript(string text)
    signal answer(string text)
    signal overlayAction(int action)
    // Keep the audio document attached to the live overlay window even when
    // the separate workbench is closed.
    width: 1; height: 1; visible: true; opacity: 0.01
    lifecycleState: WebEngineView.Active
    settings.playbackRequiresUserGesture: false
    settings.localContentCanAccessFileUrls: false
    settings.javascriptCanOpenWindows: false
    function context() {
        return JSON.stringify({game:session.game, screen:session.screen, current:session.current, cursor:session.navigation?.cursor, location:session.navigation?.location, location_status:session.navigation?.location?.status,
            guidance:session.navigation?.guidance, plan_recovery:session.navigation?.recovery, journal:session.journal, spatial:session.navigation?.spatial, coaching:session.navigation?.coaching, guide_id:session.guide?.id, current_place:session.navigation?.map?.current_place, target_place:session.navigation?.map?.target_place,
            goals:session.guide?.goals || [], coverage_notes:(session.guide?.coverage_notes || []).slice(-6),
            preparing:buddy.preparing, guide_ready:!!session.guide && session.reference_count>0, guide_revision:session.guide?.revision, progress_epoch:session.progress?.epoch, microphone_active:voice.active, returning_player:returningPlayer,
            conversation_active:conversationActive || localConversationActive || voice.speaking || buddy.busy,
            player_speaking:userSpeaking, fast_event:session.fast_event,
            instruction:"This is automatic game-screen context, not a player request. GameBuddy continuously captures the game and the configured vision model describes changes in screen.scene. Observations are estimates with timestamps. get_game_screen returns the actual latest screenshot as IMAGE content to your reasoning tool; call it for screen questions or closer inspection before claiming you cannot see. A missing location match is not missing screen access. The application sends selected spoken event and next-action cues separately; stay quiet for routine context updates. Respect unconfirmed prerequisites. Room identity does not establish inventory, player coordinates, or completion. Never mark tasks complete from this update."})
    }
    function update() {
        if (!loaded || reconnecting) return
        if(started && voiceGame!==session.game){
            recoveryCue="";recoveryAttempts=0
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
            instruction:"Background reference only. Do not infer gameplay advice from this summary. Delegate requests needing current game evidence or application actions to the backend. You can handle ordinary conversation directly. Never speak the player's side of a conversation."})
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
        const currentContext=context()
        if(trace){
            const c=JSON.parse(currentContext)
            const state=JSON.stringify({action:c.guidance?.now?.id, guidance:c.guidance?.status, screen:c.screen?.status, fresh:c.screen?.scene?.fresh, talking:c.conversation_active, busy:buddy.busy, voice:voice.speaking, backend:backendBusy, waiting:waitingForReply, assistant:assistantSpeaking})
            if(state!==lastNarrationState){lastNarrationState=state;console.warn("GAMEBUDDY_NARRATION_STATE "+state)}
        }
        const serialized=transport.next_narration(currentContext)
        if(serialized.length){
            const cue=JSON.parse(serialized),text=cue.text
            if(trace)console.warn("GAMEBUDDY_NARRATION characters="+text.length)
            if(enabledBackend){
                // This cue already contains the selected words. Appending
                // commentary here can race and cancel its queued speech.
                // Authorize playback before the cue can produce remote audio.
                speakApproved(text,cue.urgent,session.screen?.status==="live",cue.urgent?"":(session.navigation?.guidance?.now?.id||""))
            } else panel.answer(text)
        }
    }
    Timer { interval: 250; repeat: true; running: panel.automaticSession; onTriggered: panel.narrate() }
    Timer { interval: 200; repeat: true; running: panel.started && (panel.assistantSpeaking || panel.waitingForReply); onTriggered: transport.check_speech() }
    // Keep perception updates out of the live speech session. Appending
    // context while the player speaks can replace part of the transcript.
    // The reasoning worker supplies current evidence after recognition.
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
            else if(event.method==="gamebuddy/voice-mode"){
                panel.conversational=p.conversational===true
                panel.runJavaScript("gamebuddyVoice.setConversationMode("+panel.conversational+")")
            }
            else if(event.method==="gamebuddy/reasoning"){
                panel.backendBusy=p.active===true
                panel.runJavaScript("gamebuddyVoice.setBackendBusy("+panel.backendBusy+")")
            }
            else if(event.method==="thread/realtime/sdp")panel.runJavaScript("gamebuddyVoice.answer("+JSON.stringify(p.sdp)+")")
            else if(event.method==="thread/realtime/error")panel.fail(p.message || "Voice connection failed")
            else if(event.method==="thread/realtime/closed")panel.fail("Voice session ended")
            else if(event.method==="gamebuddy/speech")panel.speakApproved(p.text || "",false,!!p.step_id,p.step_id||p.discussed_step_id||"",p.delegation_id||null)
            else if(event.method==="gamebuddy/presence"){
                if(panel.backendBusy || panel.workPending)panel.runJavaScript("gamebuddyVoice.queueAside("+JSON.stringify(p.text)+")")
                else panel.speakApproved(p.text)
            }
            else if(event.method==="gamebuddy/local-speech")panel.runJavaScript("gamebuddyVoice.queuePrepared("+JSON.stringify(p.key)+","+JSON.stringify(p.text)+","+JSON.stringify(p)+")")
            else if(event.method==="gamebuddy/speech-expired"){if(panel.trace)console.warn("GAMEBUDDY_VOICE_SCOPE_EXPIRED at_ms="+Date.now());panel.pendingCue="";transport.cancel_speech();panel.answer("The view or plan changed, so I stopped the previous instruction.");panel.runJavaScript("gamebuddyVoice.interruptOutput();gamebuddyVoice.finishLocalRequest()");panel.buddy.poll_widgets()}
            else if(event.method==="gamebuddy/local-speech-error"){
                console.warn("Prepared speech unavailable: "+p.error)
                transport.cancel_speech()
                if(p.index===0)panel.runJavaScript("gamebuddyVoice.interruptOutput();gamebuddyVoice.queueSpeech("+JSON.stringify(p.text)+")")
                else {panel.runJavaScript("gamebuddyVoice.interruptOutput();gamebuddyVoice.finishLocalRequest()");panel.status="Audio unavailable · complete answer shown as text"}
            }
            else if(event.method==="gamebuddy/state-changed")panel.buddy.poll_widgets()
            else if(event.method==="gamebuddy/overlay-action"){
                panel.overlayAction(p.action)
                panel.runJavaScript("gamebuddyVoice.finishLocalRequest()")
            }
            else if(event.method==="gamebuddy/reply-error"){
                const text=p.text || "Request failed; please try again."
                panel.answer(text)
                panel.backendBusy=false;panel.waitingForReply=false
                panel.runJavaScript("gamebuddyVoice.requestFailed("+JSON.stringify(text)+")",function(){panel.speakApproved(text)})
            }
            else if(event.method==="thread/realtime/transcript/done") {
                if(p.role==="user")panel.transcript(p.text)
                else if(p.role==="assistant" && panel.conversational && p.text?.length)panel.answer(p.text)
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
        if(["capture","input-active","output-active","reply-pending","cue-timeout","connection","ready","work-pending"].includes(event.type))transport.trace_event(event.type,JSON.stringify(event.value))
        if(trace && !["input-pcm","input-text","input-final","output-active","offer"].includes(event.type))console.warn("GAMEBUDDY_VOICE_EVENT "+JSON.stringify(event))
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
            if(userSpeaking){transport.cancel_speech();pendingCue="";recoveryCue="";recoveryAttempts=0;if(!captionStreaming)captionText=""}
            transport.player_speech(userSpeaking)
        }
        else if(event.type==="input-text" || event.type==="input-final")transport.recognize_input(event.value,event.type==="input-final")
        else if(event.type==="delegation")transport.delegate_request(JSON.stringify(event.value))
        else if(event.type==="speech-ready")transport.speak(event.value)
        else if(event.type==="prepared-start" || event.type==="prepared-text")transport.record_prepared(event.value)
        else if(event.type==="prepared-error"){
            console.warn("Prepared audio playback failed: "+event.value.error)
            transport.cancel_speech()
            if(event.value.index===0)runJavaScript("gamebuddyVoice.queueSpeech("+JSON.stringify(event.value.text)+")")
            else {runJavaScript("gamebuddyVoice.finishLocalRequest()");status="Audio unavailable · complete answer shown as text"}
        }
        else if(event.type==="caption"){
            captionText=event.value;captionStreaming=captionText.length>0
            if(trace)console.warn("GAMEBUDDY_CAPTION_UPDATE characters="+captionText.length+" playback=true")
        }
        else if(event.type==="output-active"){
            if(event.value){pendingCue="";if(!assistantSpeaking && activeAdviceStep.length && activeAdviceStep===session.navigation?.guidance?.now?.id)transport.advice_spoken(context())}
            if(trace && event.value && !assistantSpeaking)console.warn("GAMEBUDDY_VOICE_PLAYBACK started at_ms="+Date.now())
            outputAt=Date.now();assistantSpeaking=event.value
            // Waiting for tools is independent of speech. Let the last words
            // expire after playback stops, including a native interruption.
            if(!assistantSpeaking)captionStreaming=false
        }
        else if(event.type==="work-pending")workPending=event.value
        else if(event.type==="reply-pending"){waitingForReply=event.value;if(!waitingForReply)captionStreaming=false}
        else if(event.type==="cue-retry" && pendingCue.length)transport.speak(pendingCue)
        else if(event.type==="request-error"){captionText=event.value;captionStreaming=false}
        else if(event.type==="cue-timeout"){
            const text=pendingCue
            if(text.length){captionText="Text reply · "+text;captionStreaming=false}
            pendingCue=""
            if(!recoverSpeech(text))status="Audio unavailable · text reply shown · you can still speak"
        }
        else if(event.type==="playback-allowed" || event.type==="reply-permit"){if(trace)console.warn("GAMEBUDDY_VOICE_GATE "+event.type+" "+event.value)}
        else if(event.type==="offer")transport.connect_offer(event.value,session.data_dir,panel.lastContext)
        else if(event.type==="error")fail(event.value)
        else if(event.type==="microphone-error") { if(voice.active)voice.toggle_listening();status="Microphone: "+event.value }
        else if(event.type==="ready") {
            connected=true;status=capturing?"Live voice · speak naturally":"Guide voice ready · mic off"
            transport.prepare_speech(JSON.stringify(session.navigation?.prepared_speech || []))
            if(recoveryCue.length){const text=recoveryCue;recoveryCue="";speakApproved(text)}
            else narrate()
        }
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
        // Routine navigation updates wait for the current reply to finish.
        // Player interruptions and explicit urgent cues still stop it promptly.
        if(activeAdviceStep!==session.navigation?.guidance?.now?.id)activeAdviceStep=""
        if(started)transport.prepare_speech(JSON.stringify(session.navigation?.prepared_speech || []))
        if(automaticSession)Qt.callLater(panel.narrate)

        update()
    }
}
