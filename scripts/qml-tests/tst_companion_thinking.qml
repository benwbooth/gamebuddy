import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 200; height: 100
    QtObject {
        id: buddy
        property bool busy: false
        property bool preparing: false
        property string transcript: ""
        property string answer: ""
        property string chat_error: ""
        property string message: "Ready"
    }
    QtObject {
        id: voice
        property bool active: true
        property bool listening: false
        property bool armed: true
        property string status: "Ready"
        property string error: ""
    }
    UI.Companion {
        id: companion
        buddy: buddy; voice: voice
        session: ({game: "Faxanadu", screen: {status: "waiting"}})
        realtimeEnabled: true
        realtimeCapturing: true
    }
    TestCase {
        name: "CompanionThinking"
        when: windowShown
        function init() {
            buddy.busy = false
            companion.realtimeEnabled = true
            companion.realtimeThinking = false
            companion.voiceTurnPending = false
        }
        function test_compact_voice_work_is_visible_and_clears() {
            const spinner = findChild(companion, "companionThinkingSpinner")
            verify(!spinner.visible)
            companion.realtimeThinking = true
            verify(companion.thinking)
            verify(spinner.visible)
            verify(companion.capturing)
            verify(!companion.expanded)
            waitForRendering(spinner)
            let saved = false
            companion.grabToImage(result => { saved = result.saveToFile("/tmp/gamebuddy-thinking-indicator.png") })
            tryVerify(() => saved)
            companion.realtimeThinking = false
            verify(!spinner.visible)
            verify(companion.capturing)
            companion.realtimeThinking = true
            companion.realtimeEnabled = false
            verify(!spinner.visible)
        }
        function test_text_and_queued_voice_work_share_indicator() {
            const spinner = findChild(companion, "companionThinkingSpinner")
            buddy.busy = true
            verify(spinner.visible)
            companion.voiceTurnPending = true
            buddy.busy = false
            verify(spinner.visible)
            companion.voiceTurnPending = false
            verify(!spinner.visible)
        }
    }
}
