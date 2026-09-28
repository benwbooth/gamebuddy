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
    property bool trace: false
    property string pageOrigin: ""
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
        return JSON.stringify({game:session.game, current:session.current, location_status:session.estimate?.status,
            goals:session.guide?.goals || [], coverage_notes:(session.guide?.coverage_notes || []).slice(-6),
            preparing:buddy.preparing, guide_revision:session.guide?.revision,
            instruction:"This is live game context, not a player request. Offer a brief useful next action on a newly located step; stay quiet for unchanged or uncertain observations. Never mark tasks complete from this update."})
    }
    function update() {
        if (!loaded) return
        const wanted=enabledBackend && voice.active
        if(wanted && !started) {
            started=true; status="Connecting live voice…"; lastContext=context()
            runJavaScript("gamebuddyVoice.start("+JSON.stringify(voice.device_name)+")")
        } else if(!wanted && started) {
            started=false; transport.disconnect(); runJavaScript("gamebuddyVoice.stop()")
            status="Voice stopped"
        }
    }
    function fail(message) {
        if(voice.active)voice.toggle_listening()
        update(); status=message
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
        if(!started)return
        if(event.type==="offer")transport.connect_offer(event.value,session.data_dir,panel.lastContext)
        else if(event.type==="error")fail(event.value)
        else if(event.type==="connection") {
            if(event.value==="connected")status="Live voice · speak naturally"
            else if(event.value==="failed" || event.value==="disconnected")fail("Voice connection lost; choose Listen to reconnect")
        }
    }
    onEnabledBackendChanged: update()
    onSpokenRepliesChanged: if(loaded)runJavaScript("gamebuddyVoice.muteOutput("+(!spokenReplies)+")")
    Connections { target: panel.voice; function onActiveChanged(){panel.update()} }
    onSessionChanged: {
        const next=context()
        if(started && next!==lastContext){lastContext=next;transport.update_context(next)}
    }
}
