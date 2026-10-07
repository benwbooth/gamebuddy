import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

// Run with -import scripts/qml-tests/stubs and a null PULSE_SINK; see README.
Item {
    UI.SpeechOutput { id: output; directory: "/tmp/gamebuddy-speech-test" }
    TestCase {
        name: "SpeechOutput"; when: windowShown
        function neural() { return findChild(output, "neuralVoice") }
        function chunk(index, last, text) {
            neural().event_json = JSON.stringify({kind:"chunk",sequence:1,index:index,last:last,text:text,url:"file:///tmp/gamebuddy-speech-test/"+index+".wav"})
        }
        function init() {
            findChild(output, "systemSpeech").volume = 0
            output.stop(); neural().available = true; neural().requests = []
        }
        function test_plays_sentences_in_order_even_when_they_arrive_out_of_order() {
            output.say("Head left through the town gate. Buy the long sword.")
            compare(neural().requests.length, 1)
            verify(output.speaking)
            chunk(1, true, "Buy the long sword.")
            compare(output.nextIndex, 0, "Second sentence must wait for the first")
            chunk(0, false, "Head left through the town gate.")
            compare(output.nextIndex, 1)
            const player = findChild(output, "speechPlayer")
            verify(String(player.source).endsWith("/0.wav"))
            tryCompare(output, "nextIndex", 2, 15000)
            verify(String(player.source).endsWith("/1.wav"))
            tryCompare(output, "speaking", false, 15000)
        }
        function test_missing_model_uses_system_speech() {
            neural().available = false
            output.say("Talk to the guru.")
            verify(!output.neuralActive)
            compare(neural().requests.length, 0)
        }
        function test_synthesis_error_finishes_reply_with_system_voice() {
            output.say("First part. Second part.")
            neural().event_json = JSON.stringify({kind:"error",sequence:1,index:0,error:"Speech synthesis timed out"})
            verify(!output.neuralActive)
        }
        function test_stop_silences_and_ignores_late_chunks() {
            output.say("One. Two.")
            output.stop()
            tryCompare(output, "speaking", false)
            chunk(0, false, "One.")
            compare(output.nextIndex, 0)
        }
    }
}
