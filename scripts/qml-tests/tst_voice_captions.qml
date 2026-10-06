import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 1280; height: 720
    UI.VoiceCaptions { id: captions; anchors.centerIn: parent }
    TestCase {
        name: "VoiceCaptions"
        when: windowShown
        function init() { captions.text="";captions.active=false;captions.lingering=false }
        function test_streaming_words_are_visible_without_waiting_for_completion() {
            captions.active=true;captions.text="Go through"
            verify(captions.visible)
            compare(findChild(captions,"liveVoiceCaption").text,"Go through")
            captions.text+=" the door on your right."
            compare(findChild(captions,"liveVoiceCaption").text,"Go through the door on your right.")
            captions.active=false
            verify(captions.visible,"Keep the last sentence available to read")
        }
        function test_long_speech_keeps_a_bounded_readable_tail() {
            captions.active=true;captions.text="Old words ".repeat(80)+"The silver lantern glows."
            verify(captions.excerpt.length <= 242)
            verify(captions.excerpt.endsWith("The silver lantern glows."))
            verify(captions.height < 200)
        }
        function test_game_text_is_never_rich_text() {
            captions.text="Use <the left door> & wait."
            compare(findChild(captions,"liveVoiceCaption").textFormat,Text.PlainText)
            captions.text=""
            verify(!captions.visible)
        }
        function test_last_words_expire_after_speech_stops() {
            captions.active=true;captions.text="Back at the green-stone passage,"
            captions.active=false
            verify(captions.visible,"Leave the interrupted words briefly readable")
            tryCompare(captions,"visible",false,8500)
            compare(captions.text,"Back at the green-stone passage,")
        }
    }
}
