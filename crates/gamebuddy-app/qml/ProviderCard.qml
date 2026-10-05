import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: card
    required property var backend
    required property string role
    required property string heading
    required property string description
    property var config: ({kind: "ollama", endpoint: "http://127.0.0.1:11434", model: "", api_key_env: "", reasoning_effort: ""})
    property var models: []
    property string defaultModel: ""
    property string requestId: ""
    property string pendingOperation: ""
    property string status: ""
    property bool failed: false
    property int generation: 0
    readonly property bool subscription: config.kind === "codex" || config.kind === "claude_code"
    readonly property var providers: [
        {id:"codex", label:"Codex · ChatGPT subscription"},
        {id:"claude_code", label:"Claude · Claude Code subscription"},
        {id:"ollama", label:"Ollama · local models"},
        {id:"open_ai_compatible", label:"OpenAI-compatible API"},
        {id:"open_ai_responses", label:"OpenAI Responses API"},
        {id:"gemini", label:"Gemini API"}
    ]
    readonly property var selected: models.find(m => m.id === (config.model || defaultModel)) || ({})
    readonly property var effortOptions: {
        const options = [{id:"auto",label:"Provider default"}]
        if (!config.reasoning_effort) options.unshift({id:"",label: config.kind === "codex" ? "Low · current default" : config.kind === "ollama" ? "Off · current default" : "Current provider default"})
        let levels = selected.efforts || []
        if (!models.length && config.kind === "claude_code" && config.model !== "haiku") levels = ["low","medium","high","max"]
        levels.forEach(level => options.push({id:level,label:level === "off" ? "Off · fastest" : level === "on" ? "On" : level.charAt(0).toUpperCase()+level.slice(1)}))
        if (config.reasoning_effort && !options.some(o => o.id === config.reasoning_effort)) options.push({id:config.reasoning_effort,label:config.reasoning_effort+" · saved setting"})
        return options
    }
    signal edited()
    onConfigChanged: {
        if (modelName) modelName.text = config.model || ""
        if (endpoint) endpoint.text = config.endpoint || ""
        if (keyName) keyName.text = config.api_key_env || ""
    }
    implicitHeight: content.implicitHeight + 40
    color: "#172332"; radius: 14; border.color: "#344355"

    function setField(name, value) {
        const next = Object.assign({}, config); next[name] = value; config = next
        edited()
    }
    function invalidate() { generation++; requestId = ""; pendingOperation = ""; status = "" }
    function fastEffort(model) {
        const levels = (models.find(m => m.id === (model || defaultModel)) || {}).efforts || []
        if (levels.indexOf("off") >= 0) return "off"
        if (levels.indexOf("low") >= 0) return "low"
        if (config.kind === "codex" && !model) return "low"
        return "auto"
    }
    function load(value) { invalidate(); config = Object.assign({}, value); models = []; defaultModel = "" }
    function configuration() {
        return Object.assign({}, config, {endpoint:subscription ? "" : endpoint.text.trim(), api_key_env:subscription ? "" : keyName.text.trim(), model:modelName.text.trim()})
    }
    function request(operation) {
        config = configuration()
        requestId = role + ":" + (++generation)
        pendingOperation = operation; failed = false
        status = operation === "models" ? "Finding models…" : "Waiting for a short test reply…"
        backend.provider_request(requestId, JSON.stringify(config), operation)
    }
    Connections {
        target: card.backend
        function onProvider_result(id, result) {
            if (id !== card.requestId) return
            const response = JSON.parse(result), operation = card.pendingOperation
            card.pendingOperation = ""; card.failed = !!response.error
            if (response.error) { card.status = response.error; return }
            if (operation === "models") {
                card.models = response.models || []; card.defaultModel = response.default_model || ""
                card.status = (card.defaultModel ? "GameBuddy default: " + card.defaultModel + ". " : "") + (response.note || "")
            } else {
                card.status = "Connected · reply in " + (response.elapsed_ms / 1000).toFixed(2) + " s · " + response.reply
            }
        }
    }
    ColumnLayout {
        id: content
        anchors { left:parent.left; right:parent.right; top:parent.top; margins:20 }
        spacing: 12
        Text { text:card.heading; color:"#eaf2fa"; font.pixelSize:20; font.bold:true }
        Text { text:card.description; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true }
        ComboBox {
            id: providerChoice; objectName:card.role+"Provider"
            Layout.fillWidth:true; model:card.providers; textRole:"label"; valueRole:"id"
            currentIndex:card.providers.findIndex(p => p.id === card.config.kind)
            onActivated: {
                const kind = currentValue
                card.invalidate(); card.models = []; card.defaultModel = ""
                card.config = {kind:kind,model:kind === "claude_code" ? "sonnet" : "",endpoint:kind === "ollama" ? "http://127.0.0.1:11434" : kind === "open_ai_responses" || kind === "open_ai_compatible" ? "https://api.openai.com/v1" : kind === "gemini" ? "https://generativelanguage.googleapis.com/v1beta" : "",api_key_env:"",reasoning_effort:kind === "codex" ? "low" : ""}
                card.edited(); card.request("models")
            }
        }
        Text { visible:card.subscription; text:card.config.kind === "codex" ? "Uses your existing Codex CLI login (codex login)." : "Uses your existing Claude Code login (claude auth login)."; color:"#a3b4c7"; font.pixelSize:12; wrapMode:Text.WordWrap; Layout.fillWidth:true }
        GbField {
            id:endpoint; objectName:card.role+"Endpoint"; visible:!card.subscription; Layout.fillWidth:true
            text:card.config.endpoint || ""; placeholderText:"Server address"
            onEditingFinished: if (text.trim() !== card.config.endpoint) { card.invalidate(); card.models=[]; card.setField("endpoint",text.trim()) }
        }
        RowLayout {
            Layout.fillWidth:true
            GbField {
                id:modelName; objectName:card.role+"Model"; Layout.fillWidth:true
                text:card.config.model || ""; placeholderText:card.config.kind === "codex" ? "GameBuddy default · GPT-6 Luna" : card.subscription ? "Client default" : "Model name"
                onEditingFinished: if (text.trim() !== card.config.model) { card.invalidate(); card.setField("model",text.trim()); card.setField("reasoning_effort",card.fastEffort(text.trim())) }
            }
            GbButton { text:"Find models"; enabled:!card.pendingOperation; onClicked:card.request("models") }
        }
        ComboBox {
            objectName:card.role+"ModelPicker"; visible:card.models.length > 0; Layout.fillWidth:true
            model:card.models; textRole:"label"; valueRole:"id"
            currentIndex:card.models.findIndex(m => m.id === card.config.model)
            displayText:currentIndex < 0 ? "Choose an available model…" : currentText
            onActivated: { card.invalidate(); card.setField("model",currentValue); card.setField("reasoning_effort",card.fastEffort(currentValue)) }
        }
        RowLayout {
            Layout.fillWidth:true
            Text { text:"Thinking"; color:"#eaf2fa"; Layout.preferredWidth:80 }
            ComboBox {
                objectName:card.role+"Thinking"; Layout.fillWidth:true
                model:card.effortOptions; textRole:"label"; valueRole:"id"
                currentIndex:card.effortOptions.findIndex(o => o.id === (card.config.reasoning_effort || ""))
                onActivated: { card.invalidate(); card.setField("reasoning_effort",currentValue) }
            }
        }
        Text { text:"Lower thinking usually answers sooner. Available controls depend on the model."; color:"#a3b4c7"; font.pixelSize:12; Layout.fillWidth:true; wrapMode:Text.WordWrap }
        Text { visible:card.role === "vision" && card.selected.vision === false; text:"This model does not accept images. Choose a vision model for screen interpretation."; color:"#ffbd8b"; Layout.fillWidth:true; wrapMode:Text.WordWrap }
        GbField {
            id:keyName; visible:!card.subscription && card.config.kind !== "ollama"; Layout.fillWidth:true
            text:card.config.api_key_env || ""; placeholderText:"API key environment variable, e.g. OPENAI_API_KEY"
            onEditingFinished:card.setField("api_key_env",text.trim())
        }
        GbButton { objectName:card.role+"Test"; text:card.pendingOperation === "test" ? "Testing…" : "Test connection"; enabled:!card.pendingOperation; onClicked:card.request("test") }
        Text { visible:!!card.status; text:card.status; color:card.failed ? "#ffb0a4" : "#86decb"; font.pixelSize:12; Layout.fillWidth:true; wrapMode:Text.WordWrap; textFormat:Text.PlainText }
    }
}
