import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 450; height: 600
    QtObject {
        id: buddy
        property bool busy: false
        property bool preparing: false
        property string transcript: ""
        property string answer: ""
        property string chat_error: ""
        property string message: "Ready"
        property var requests: []
        property bool accept: true
        function ask(text) {
            if (!accept || busy) return false
            requests.push(text)
            transcript = text
            busy = true
            return true
        }
    }
    QtObject {
        id: voice
        property bool active: false
        property bool listening: false
        property bool armed: false
        property string status: "Ready"
        property string error: ""
        function toggle_listening() { active = !active }
    }
    UI.Companion {
        id: companion
        buddy: buddy
        voice: voice
        session: ({game: "Faxanadu", screen: {status: "waiting"}})
        expanded: true
    }
    TestCase {
        name: "CompanionChat"
        when: windowShown
        function init() {
            buddy.busy = false; buddy.accept = true; buddy.requests = []
            buddy.transcript = ""; buddy.answer = ""; buddy.chat_error = ""
            companion.voiceTurnPending = false; companion.expanded = true
            findChild(companion, "companionChatInput").text = ""
        }
        function test_enter_sends_trimmed_text_once_without_microphone() {
            const input = findChild(companion, "companionChatInput")
            const send = findChild(companion, "companionChatSend")
            verify(!send.enabled)
            mouseClick(input)
            verify(input.activeFocus)
            input.text = "   Where am I?   "
            verify(send.enabled)
            keyClick(Qt.Key_Return)
            compare(buddy.requests.length, 1)
            compare(buddy.requests[0], "Where am I?")
            compare(input.text, "")
            compare(voice.active, false)
            compare(companion.recording, false)
            input.text = "A follow-up"
            keyClick(Qt.Key_Return)
            compare(buddy.requests.length, 1)
            compare(input.text, "A follow-up")
            verify(!send.enabled)
        }
        function test_send_button_and_rejected_draft() {
            const input = findChild(companion, "companionChatInput")
            const send = findChild(companion, "companionChatSend")
            input.text = "Keep this draft"
            buddy.accept = false
            mouseClick(send)
            compare(input.text, "Keep this draft")
            buddy.accept = true
            companion.voiceTurnPending = true
            verify(!send.enabled)
            companion.sendMessage()
            compare(buddy.requests.length, 0)
            companion.voiceTurnPending = false
            mouseClick(send)
            compare(buddy.requests.length, 1)
            compare(input.text, "")
        }
        function test_whitespace_and_collapsed_focus() {
            const input = findChild(companion, "companionChatInput")
            const send = findChild(companion, "companionChatSend")
            input.text = "  "
            verify(!send.enabled)
            companion.sendMessage()
            compare(buddy.requests.length, 0)
            companion.expanded = false
            verify(!input.visible)
            verify(!send.visible)
        }
        function test_error_is_visible_as_plain_text() {
            buddy.transcript = "Where am I?"
            buddy.chat_error = "Connection failed: <try again>"
            const reply = findChild(companion, "companionChatReply")
            compare(reply.text, buddy.chat_error)
            compare(reply.textFormat, Text.PlainText)
        }
        function test_layout() {
            buddy.transcript = "Can you help me find the next town?"
            buddy.answer = "Follow the road east. I can use the map once tracking has a reliable match."
            const input = findChild(companion, "companionChatInput")
            const p = input.mapToItem(companion, 0, 0)
            verify(p.y >= 0 && p.y + input.height < companion.height)
            verify(input.width > 200)
            waitForRendering(companion)
            grabImage(companion).save("/tmp/gamebuddy-text-companion.png")
        }
    }
}
