import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 400; height: 200
    UI.TokenAlert { id: alert; width: 380 }
    SignalSpy { id: dismissed; target: alert; signalName: "dismissed" }
    TestCase {
        name: "TokenAlert"; when: windowShown
        function test_shows_the_provider_message_and_dismisses() {
            alert.message = ""
            verify(!alert.visible)
            alert.message = "Out of tokens: Gemini has no tokens or quota left. You exceeded your current quota."
            verify(alert.visible)
            const text = findChild(alert, "tokenAlertMessage").text
            verify(text.startsWith("Gemini has no tokens"), text)
            mouseClick(findChild(alert, "tokenAlertDismiss"))
            compare(dismissed.count, 1)
            waitForRendering(alert)
            grabImage(alert).save("/tmp/gamebuddy-token-alert.png")
        }
        function test_voice_outage_has_its_own_title() {
            alert.message = "Voice unavailable: Gemini Live is not responding (Internal error encountered.). GameBuddy is using the local voice."
            compare(findChild(alert, "tokenAlertTitle").text, "Voice unavailable")
            verify(findChild(alert, "tokenAlertMessage").text.startsWith("Gemini Live is not responding"))
        }
    }
}
