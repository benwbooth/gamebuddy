import QtQuick
import QtQuick.Controls
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width:1240; height:900
    QtObject {
        id:backend
        property bool busy:false
        property string settings_json:JSON.stringify({reasoning:{kind:"codex",endpoint:"",model:"",api_key_env:""},vision:{kind:"ollama",endpoint:"http://127.0.0.1:11434",model:"qwen3.5:4b",api_key_env:""},speech:"codex_realtime",duck_game_audio:true,fast_voice:true})
        property var requests:[]
        signal provider_result(string id,string result)
        function provider_request(id,config,operation) { requests.push({id:id,config:JSON.parse(config),operation:operation}) }
        function save_settings(text) { settings_json = text }
    }
    UI.ModelSettings { id:page; anchors.fill:parent; backend:backend }
    TestCase {
        name:"ModelSettings"; when:windowShown
        function init() {
            backend.settings_json=JSON.stringify({single_provider:false,reasoning:{kind:"codex",endpoint:"",model:"",api_key_env:""},vision:{kind:"ollama",endpoint:"http://127.0.0.1:11434",model:"qwen3.5:4b",api_key_env:""},speech:"codex_realtime",duck_game_audio:true,fast_voice:true})
            page.reload();wait(20);backend.requests=[]
        }
        function test_save_model_thinking_and_voice_without_json() {
            const provider = findChild(page,"reasoningProvider")
            provider.currentIndex=1; provider.activated(1)
            const model=findChild(page,"reasoningModel")
            compare(model.text,"sonnet")
            const request=backend.requests[backend.requests.length-1]
            backend.provider_result(request.id,JSON.stringify({models:[{id:"sonnet",label:"Sonnet",efforts:["low","medium","high"]}]}))
            const thinking=findChild(page,"reasoningThinking")
            thinking.currentIndex=thinking.model.findIndex(o=>o.id==="low"); thinking.activated(thinking.currentIndex)
            const voice=findChild(page,"speechBackend")
            voice.currentIndex=0;voice.activated(0)
            mouseClick(findChild(page,"saveModels"))
            const saved=JSON.parse(backend.settings_json)
            compare(saved.reasoning.kind,"claude_code");compare(saved.reasoning.model,"sonnet")
            compare(saved.reasoning.reasoning_effort,"low");compare(saved.speech,"local")
            compare(saved.vision.model,"qwen3.5:4b");verify(saved.duck_game_audio)
            verify(!page.dirty)
        }
        function test_stale_discovery_cannot_replace_new_provider() {
            const provider=findChild(page,"reasoningProvider")
            provider.currentIndex=0;provider.activated(0)
            const old=backend.requests[backend.requests.length-1].id
            provider.currentIndex=2;provider.activated(2)
            const current=backend.requests[backend.requests.length-1].id
            backend.provider_result(current,JSON.stringify({models:[{id:"local:7b",label:"local:7b",efforts:["off","on"],vision:true}]}))
            backend.provider_result(old,JSON.stringify({models:[{id:"wrong",label:"Wrong",efforts:["high"]}]}))
            const picker=findChild(page,"reasoningModelPicker")
            compare(picker.count,1);compare(picker.model[0].id,"local:7b")
            picker.currentIndex=0;picker.activated(0)
            const thinking=findChild(page,"reasoningThinking")
            compare(thinking.count,3)
            thinking.currentIndex=thinking.model.findIndex(o=>o.id==="off");thinking.activated(thinking.currentIndex)
            page.save()
            compare(JSON.parse(backend.settings_json).reasoning.reasoning_effort,"off")
        }
        function test_connection_uses_unsaved_values_and_reports_error() {
            const model=findChild(page,"reasoningModel")
            model.text="custom:9b";model.editingFinished()
            mouseClick(findChild(page,"reasoningTest"))
            const request=backend.requests[backend.requests.length-1]
            compare(request.operation,"test");compare(request.config.model,"custom:9b")
            verify(!findChild(page,"reasoningTest").enabled)
            backend.provider_result(request.id,JSON.stringify({error:"Server unavailable"}))
            verify(findChild(page,"reasoningTest").enabled)
            verify(page.dirty)
        }
        function test_anthropic_api_has_independent_key_model_and_voice() {
            const provider=findChild(page,"reasoningProvider")
            provider.currentIndex=provider.model.findIndex(p=>p.id==="anthropic");provider.activated(provider.currentIndex)
            compare(findChild(page,"reasoningEndpoint").text,"https://api.anthropic.com/v1")
            compare(findChild(page,"reasoningKeyName").text,"ANTHROPIC_API_KEY")
            const request=backend.requests[backend.requests.length-1]
            compare(request.config.kind,"anthropic");compare(request.config.model,"")
            backend.provider_result(request.id,JSON.stringify({models:[{id:"claude-fixture",label:"Claude fixture",efforts:["low","high"],vision:true}]}))
            const picker=findChild(page,"reasoningModelPicker")
            picker.currentIndex=0;picker.activated(0)
            const voice=findChild(page,"speechBackend")
            voice.currentIndex=1;voice.activated(1)
            page.save()
            const saved=JSON.parse(backend.settings_json)
            compare(saved.reasoning.model,"claude-fixture");compare(saved.reasoning.reasoning_effort,"low")
            compare(saved.reasoning.api_key_env,"ANTHROPIC_API_KEY");compare(saved.speech,"codex_realtime")
            // Moving back to the subscription cannot retain API credentials.
            provider.currentIndex=1;provider.activated(1);page.save()
            const subscription=JSON.parse(backend.settings_json).reasoning
            compare(subscription.kind,"claude_code");compare(subscription.model,"sonnet")
            compare(subscription.endpoint,"");compare(subscription.api_key_env,"")
        }
        function test_claude_takes_every_role_and_replaces_codex_voice() {
            const provider=findChild(page,"reasoningProvider")
            provider.currentIndex=1;provider.activated(1)
            const scope=findChild(page,"singleProvider")
            scope.checked=true;scope.clicked()
            const voice=findChild(page,"speechBackend")
            compare(voice.currentIndex,0)
            page.save()
            let saved=JSON.parse(backend.settings_json)
            verify(saved.single_provider)
            compare(saved.reasoning.kind,"claude_code");compare(saved.vision.kind,"claude_code")
            compare(saved.vision.model,"sonnet");compare(saved.planning,null);compare(saved.speech,"local")
            // Later model/provider changes must update every role, including audio.
            const model=findChild(page,"reasoningModel")
            model.text="opus";model.editingFinished();page.save()
            saved=JSON.parse(backend.settings_json)
            compare(saved.vision.model,"opus")
            provider.currentIndex=0;provider.activated(0);page.save()
            saved=JSON.parse(backend.settings_json)
            compare(saved.vision.kind,"codex");compare(saved.speech,"codex_realtime")
            provider.currentIndex=6;provider.activated(6)
            model.text="claude-fixture";model.editingFinished();page.save()
            saved=JSON.parse(backend.settings_json)
            compare(saved.vision.kind,"anthropic");compare(saved.vision.api_key_env,"ANTHROPIC_API_KEY")
            compare(saved.speech,"local")
            // Gemini Live only voices the assistant, so it stays selected.
            voice.currentIndex=2;voice.activated(2);page.save()
            saved=JSON.parse(backend.settings_json)
            compare(saved.speech,"gemini_live");compare(saved.vision.kind,"anthropic")
            compare(saved.live_voice.model,"gemini-3.8-live");compare(saved.live_voice.api_key_env,"GEMINI_API_KEY")
        }
    }
}
