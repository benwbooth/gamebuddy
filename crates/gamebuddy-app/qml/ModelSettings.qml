import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ScrollView {
    id: page
    required property var backend
    property string audioStatus: ""
    property bool dirty: false
    property bool loaded: false
    clip:true; contentWidth:availableWidth
    palette.windowText:"#eaf2fa"; palette.text:"#eaf2fa"; palette.buttonText:"#eaf2fa"
    palette.button:"#28394d"; palette.base:"#101c2a"; palette.highlight:"#337b70"; palette.highlightedText:"#ffffff"
    function reload() {
        const settings = JSON.parse(backend.settings_json)
        if (!settings.reasoning || !settings.vision) return
        reasoning.load(settings.reasoning); vision.load(settings.vision)
        automaticPlanning.checked = !settings.planning
        planning.load(settings.planning || (settings.reasoning.kind === "codex" ? Object.assign({},settings.reasoning,{model:"gpt-6.1-sol", reasoning_effort:"medium"}) : settings.reasoning))
        voice.currentIndex = settings.speech === "codex_realtime" ? 1 : 0
        fast.checked = settings.fast_voice !== false; duck.checked = settings.duck_game_audio !== false
        preparedVoice.enabled=!!settings.prepared_speech
        preparedVoice.checked=!!settings.prepared_speech && settings.prepared_speech.enabled!==false
        dirty = false; loaded = true
        if (visible) Qt.callLater(discover)
    }
    function discover() {
        if (!visible || !loaded) return
        if (!reasoning.models.length && !reasoning.pendingOperation) reasoning.request("models")
        if (!vision.models.length && !vision.pendingOperation) vision.request("models")
        if (!automaticPlanning.checked && !planning.models.length && !planning.pendingOperation) planning.request("models")
    }
    function save() {
        const settings = JSON.parse(backend.settings_json)
        settings.reasoning = reasoning.configuration(); settings.vision = vision.configuration()
        settings.planning = automaticPlanning.checked ? null : planning.configuration()
        settings.speech = voice.currentIndex === 1 ? "codex_realtime" : "local"
        settings.fast_voice = fast.checked; settings.duck_game_audio = duck.checked
        if(settings.prepared_speech)settings.prepared_speech.enabled=preparedVoice.checked
        backend.save_settings(JSON.stringify(settings))
    }
    Component.onCompleted:reload()
    onVisibleChanged: if (visible) Qt.callLater(discover)
    Connections { target:page.backend; function onSettings_jsonChanged() { page.reload() } }
    ColumnLayout {
        width:page.availableWidth; spacing:16
        Text { text:"Models & speed"; color:"#eaf2fa"; font.pixelSize:28; Layout.margins:24; Layout.bottomMargin:0 }
        Text { text:"Choose who answers and how much time they spend thinking. Map tracking runs independently on this computer."; color:"#a3b4c7"; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24 }
        RowLayout {
            Layout.leftMargin:24
            GbButton { objectName:"saveModels"; text:page.dirty ? "Save changes" : "Settings saved"; accent:true; enabled:page.dirty && !page.backend.busy; onClicked:page.save() }
            GbButton { text:"Discard changes"; enabled:page.dirty; onClicked:page.reload() }
        }
        ProviderCard {
            id:reasoning; backend:page.backend; role:"reasoning"; heading:"Interactive advice"
            description:"Answers player questions and resolves corrections in text and voice. Lower thinking reduces response time."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:page.dirty = true
        }
        ProviderCard {
            id:vision; backend:page.backend; role:"vision"; heading:"Screen interpretation"
            description:"Reads screenshots when the guide needs a visual check. Choose a model that accepts images."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:page.dirty = true
        }
        CheckBox {
            id:automaticPlanning; objectName:"automaticPlanning"; text:"Choose background planning model automatically"
            Layout.leftMargin:24; checked:true; onClicked:{page.dirty=true; if(!checked) page.discover()}
        }
        Text {
            text:"Background planning prepares reusable route rules. With Codex advice, automatic planning uses GPT-6.1 Sol at medium thinking. Inventory and route updates then run locally."
            color:"#a3b4c7"; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
        }
        ProviderCard {
            id:planning; backend:page.backend; role:"planning"; heading:"Background route planning"
            visible:!automaticPlanning.checked
            description:"Interprets requirements and optional branches ahead of play without delaying voice or map updates."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:page.dirty=true
        }
        Text { text:"Voice"; color:"#eaf2fa"; font.pixelSize:20; Layout.leftMargin:24 }
        ComboBox {
            id:voice; objectName:"speechBackend"; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            model:["Local speech · uses selected advice provider","Codex live voice · ChatGPT subscription"]
            onActivated:page.dirty = true
        }
        Text { text:"Both voice modes use the selected advice provider. Codex live voice still uses your ChatGPT subscription for listening and speech; choose Local speech to use local audio. Reconnect voice after saving changes."; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24 }
        CheckBox { id:fast; text:"Faster Codex voice replies (higher quota use)"; visible:voice.currentIndex === 1 && reasoning.config.kind === "codex"; Layout.leftMargin:24; onClicked:page.dirty=true }
        CheckBox { id:preparedVoice; text:"Prepared neural speech · local voice"; visible:voice.currentIndex===1; Layout.leftMargin:24; onClicked:page.dirty=true }
        Text { visible:voice.currentIndex===1; text:preparedVoice.enabled ? "Uses the installed local voice for faster replies and prepares upcoming instructions. Codex continues listening. Reconnect after changing this setting." : "Install a prepared speech voice to enable local audio preparation."; color:"#a3b4c7";font.pixelSize:12;wrapMode:Text.WordWrap;Layout.fillWidth:true;Layout.leftMargin:24;Layout.rightMargin:24 }
        CheckBox { id:duck; text:"Lower game volume during conversations"; Layout.leftMargin:24; onClicked:page.dirty=true }
        Text { text:page.audioStatus; color:"#a3b4c7"; font.pixelSize:12; Layout.leftMargin:24; Layout.rightMargin:24; Layout.fillWidth:true; wrapMode:Text.WordWrap }
        RowLayout {
            Layout.leftMargin:24
            GbButton { text:page.dirty ? "Save changes" : "Settings saved"; accent:true; enabled:page.dirty && !page.backend.busy; onClicked:page.save() }
            GbButton { text:"Discard changes"; enabled:page.dirty; onClicked:page.reload() }
        }
        Text { text:"Cloud providers receive the text and screenshots sent to them. API keys stay in environment variables; only their variable names are saved."; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.margins:24; Layout.topMargin:0 }
    }
}
