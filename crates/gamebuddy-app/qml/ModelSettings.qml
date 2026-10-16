import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import GameBuddy

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
        singleProvider.checked = settings.single_provider === true
        reasoning.load(settings.reasoning); vision.load(settings.vision)
        automaticPlanning.checked = !settings.planning
        planning.load(settings.planning || (settings.reasoning.kind === "codex" ? Object.assign({},settings.reasoning,{model:"gpt-6.1-sol", reasoning_effort:"medium"}) : settings.reasoning))
        voice.currentIndex = settings.speech === "codex_realtime" ? 1 : settings.speech === "gemini_live" ? 2 : 0
        liveModel.text = settings.live_voice?.model || "gemini-3.8-live"
        liveVoiceName.text = settings.live_voice?.voice || "Achird"
        liveThinking.currentIndex = Math.max(0, liveThinking.values.indexOf(settings.live_voice?.thinking_level || "high"))
        fast.checked = settings.fast_voice !== false; duck.checked = settings.duck_game_audio !== false
        preparedVoice.enabled=!!settings.prepared_speech
        preparedVoice.checked=!!settings.prepared_speech && settings.prepared_speech.enabled!==false
        dirty = false; loaded = true
        if (visible) Qt.callLater(discover)
    }
    function discover() {
        if (!visible || !loaded) return
        if (!reasoning.models.length && !reasoning.pendingOperation) reasoning.request("models")
        if (!singleProvider.checked && !vision.models.length && !vision.pendingOperation) vision.request("models")
        if (!singleProvider.checked && !automaticPlanning.checked && !planning.models.length && !planning.pendingOperation) planning.request("models")
    }
    function applySingleProvider() {
        if (!singleProvider.checked) return
        const config = reasoning.configuration()
        vision.load(config); planning.load(config); automaticPlanning.checked = true
        // The voice backend stays an independent choice, for easy comparison.
    }
    function save() {
        applySingleProvider()
        const settings = JSON.parse(backend.settings_json)
        settings.single_provider = singleProvider.checked
        settings.reasoning = reasoning.configuration(); settings.vision = vision.configuration()
        settings.planning = automaticPlanning.checked ? null : planning.configuration()
        settings.speech = ["local", "codex_realtime", "gemini_live"][voice.currentIndex]
        settings.live_voice = Object.assign({api_key_env: "GEMINI_API_KEY"}, settings.live_voice || {}, {model: liveModel.text.trim(), voice: liveVoiceName.text.trim(), thinking_level: liveThinking.values[liveThinking.currentIndex]})
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
            Layout.leftMargin:24; Layout.rightMargin:24; spacing:8
            Text { text:"Quick setup"; color:"#a3b4c7"; font.pixelSize:12 }
            GbButton { objectName:"presetOpenAI"; text:"OpenAI (Codex)"; enabled:!page.backend.busy; onClicked:page.backend.apply_preset("openai") }
            GbButton { objectName:"presetGemini"; text:"Gemini Live"; enabled:!page.backend.busy; onClicked:page.backend.apply_preset("gemini") }
            GbButton { objectName:"presetLocal"; text:"Local (Claude + Ryan)"; enabled:!page.backend.busy; onClicked:page.backend.apply_preset("local") }
        }
        RowLayout {
            Layout.leftMargin:24
            GbButton { objectName:"saveModels"; text:page.dirty ? "Save changes" : "Settings saved"; accent:true; enabled:page.dirty && !page.backend.busy; onClicked:page.save() }
            GbButton { text:"Discard changes"; enabled:page.dirty; onClicked:page.reload() }
        }
        CheckBox {
            id:singleProvider; objectName:"singleProvider"; text:"Use one provider for all AI work"
            Layout.leftMargin:24
            onClicked:{page.dirty=true;page.applySingleProvider();page.discover()}
        }
        Text {
            visible:singleProvider.checked
            text:voice.currentIndex === 2 ? "The provider below handles text requests and guide preparation. Gemini Live handles the live conversation; Extended Thinking also handles live game reasoning directly." : "The provider below handles game advice, tools, memory recall, screenshots and background planning. The voice choice below controls listening and spoken conversation."
            color:"#a3b4c7";wrapMode:Text.WordWrap;Layout.fillWidth:true;Layout.leftMargin:24;Layout.rightMargin:24
        }
        ProviderCard {
            id:reasoning; backend:page.backend; role:"reasoning"; heading:singleProvider.checked ? "Assistant provider" : "Conversation & game advice"
            description:"Handles conversation and game requests in text and voice. Lower thinking reduces response time."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:{page.dirty = true;page.applySingleProvider()}
        }
        ProviderCard {
            id:vision; backend:page.backend; role:"vision"; heading:"Screen interpretation"
            visible:!singleProvider.checked
            description:"Reads screenshots when the guide needs a visual check. Choose a model that accepts images."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:page.dirty = true
        }
        CheckBox {
            id:automaticPlanning; objectName:"automaticPlanning"; text:"Choose background planning model automatically"
            visible:!singleProvider.checked
            Layout.leftMargin:24; checked:true; onClicked:{page.dirty=true; if(!checked) page.discover()}
        }
        Text {
            visible:!singleProvider.checked
            text:"Background planning prepares reusable route rules. With Codex advice, automatic planning uses GPT-6.1 Sol at medium thinking. Inventory and route updates then run locally."
            color:"#a3b4c7"; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
        }
        ProviderCard {
            id:planning; backend:page.backend; role:"planning"; heading:"Background route planning"
            visible:!singleProvider.checked && !automaticPlanning.checked
            description:"Interprets requirements and optional branches ahead of play without delaying voice or map updates."
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            onEdited:page.dirty=true
        }
        Text { text:"Voice"; color:"#eaf2fa"; font.pixelSize:20; Layout.leftMargin:24 }
        ComboBox {
            id:voice; objectName:"speechBackend"; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            model:["Local audio · selected provider handles all conversation","Codex live voice · ChatGPT subscription","Gemini Live · speech and visual conversation"]
            onActivated:page.dirty = true
        }
        Text { text:voice.currentIndex === 2 ? "Gemini handles live speech and sees the game. Extended Thinking reasons about game questions directly and uses the guide, map, inventory and objective tools. The assistant provider above still handles text requests and guide preparation. Reconnect voice after saving changes." : singleProvider.checked ? (reasoning.config.kind === "codex" ? "This setup uses Codex for both the assistant and live voice." : "All spoken answers come from the selected assistant. Audio is transcribed and spoken locally. Local listening pauses while processing and playing a reply; Stop cuts off playback.") : "Local audio sends every conversation to the selected provider. Codex live voice also uses ChatGPT for ordinary conversation, listening, and speech. Reconnect voice after saving changes."; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24 }
        CheckBox { id:fast; text:"Faster Codex voice replies (higher quota use)"; visible:voice.currentIndex === 1 && reasoning.config.kind === "codex"; Layout.leftMargin:24; onClicked:page.dirty=true }
        CheckBox { id:preparedVoice; text:"Prepared neural speech · local voice"; visible:voice.currentIndex===1; Layout.leftMargin:24; onClicked:page.dirty=true }
        Text { visible:voice.currentIndex===1; text:preparedVoice.enabled ? "Uses the installed local voice for faster replies and prepares upcoming instructions. Codex continues listening. Reconnect after changing this setting." : "Install a prepared speech voice to enable local audio preparation."; color:"#a3b4c7";font.pixelSize:12;wrapMode:Text.WordWrap;Layout.fillWidth:true;Layout.leftMargin:24;Layout.rightMargin:24 }
        GeminiVoice { id:geminiKeys }
        GridLayout {
            visible:voice.currentIndex===2; columns:2; columnSpacing:10; rowSpacing:8
            Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24
            Text { text:"Gemini model"; color:"#a3b4c7"; font.pixelSize:12 }
            TextField { id:liveModel; objectName:"geminiModel"; Layout.fillWidth:true; onTextEdited:page.dirty=true }
            Text { visible:liveModel.text.trim() === "gemini-3.8-live-extended-thinking"; text:"Thinking"; color:"#a3b4c7"; font.pixelSize:12 }
            ComboBox {
                id:liveThinking; objectName:"geminiThinking"; Layout.fillWidth:true
                visible:liveModel.text.trim() === "gemini-3.8-live-extended-thinking"
                readonly property var values:["low", "medium", "high"]
                model:["Low · quicker", "Medium", "High · deeper reasoning"]
                currentIndex:2; onActivated:page.dirty=true
            }
            Text { text:"Voice"; color:"#a3b4c7"; font.pixelSize:12 }
            TextField { id:liveVoiceName; objectName:"geminiVoiceName"; Layout.fillWidth:true; placeholderText:"Charon (calm), Schedar (even), Umbriel (easy-going), Sulafat (warm), Puck (upbeat)…"; onTextEdited:page.dirty=true }
            Text { text:"API key"; color:"#a3b4c7"; font.pixelSize:12 }
            RowLayout {
                Layout.fillWidth:true
                TextField { id:geminiKey; objectName:"geminiApiKey"; Layout.fillWidth:true; echoMode:TextInput.Password; placeholderText:"Paste a Google AI Studio key to replace the saved one" }
                GbButton { text:"Save key"; enabled:geminiKey.text.trim().length>0; onClicked:{ const error=geminiKeys.save_api_key(geminiKey.text); geminiKeyStatus.text=error.length ? error : "Key saved privately on this computer."; if(!error.length)geminiKey.text="" } }
            }
        }
        Text { id:geminiKeyStatus; visible:voice.currentIndex===2; text:"Gemini listens, watches the game and answers using GameBuddy's walkthrough tools; the assistant above prepares walkthroughs. The key comes from GEMINI_API_KEY, or the private key file ~/.config/gamebuddy/gemini-api-key. Reconnect voice after saving."; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true; Layout.leftMargin:24; Layout.rightMargin:24 }
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
