import QtQuick
import QtMultimedia
import QtTextToSpeech
import GameBuddy

// Spoken replies in local voice mode. Uses the installed Piper neural voice
// sentence by sentence; system speech covers a missing model or a failure.
Item {
    id: output
    property string directory: ""
    property bool trace: false
    property bool neuralActive: false
    property var chunks: ({})
    property int nextIndex: 0
    property int lastIndex: -1
    property string text: ""
    property string spoken: ""
    readonly property bool neuralAvailable: neural.available
    readonly property bool speaking: neuralActive || system.state === TextToSpeech.Speaking
    readonly property bool busy: neuralActive || (system.state !== TextToSpeech.Ready && system.state !== TextToSpeech.Error)

    onDirectoryChanged: if (directory.length) neural.configure(directory)

    function say(value) {
        stop()
        text = value
        if (directory.length && neural.configure(directory) && neural.speak(value)) {
            neuralActive = true
            return
        }
        system.say(value)
    }
    function stop() {
        neural.stop()
        neuralActive = false
        chunks = ({}); nextIndex = 0; lastIndex = -1; spoken = ""
        player.stop()
        system.stop()
    }
    function playNext() {
        const chunk = chunks[nextIndex]
        if (!chunk) {
            // Sentences can finish synthesizing out of order.
            if (lastIndex >= 0 && nextIndex > lastIndex) neuralActive = false
            return
        }
        if (player.playbackState === MediaPlayer.PlayingState) return
        nextIndex++
        spoken += (spoken.length ? " " : "") + chunk.text
        player.source = chunk.url
        player.play()
    }
    function fallBack(error) {
        if (trace) console.warn("GAMEBUDDY_NEURAL_SPEECH_ERROR " + error)
        // Finish the reply with the system voice from where neural audio stopped.
        const rest = text.indexOf(spoken) === 0 ? text.slice(spoken.length).trim() : (spoken.length ? "" : text)
        neural.stop()
        neuralActive = false
        if (rest.length) system.say(rest)
    }

    NeuralVoice {
        id: neural
        objectName: "neuralVoice"
        onEvent_jsonChanged: {
            const event = JSON.parse(event_json)
            if (!output.neuralActive) return
            if (event.kind === "error") { output.fallBack(event.error); return }
            const next = Object.assign({}, output.chunks)
            next[event.index] = event
            output.chunks = next
            if (event.last) output.lastIndex = event.index
            if (output.trace) console.warn("GAMEBUDDY_NEURAL_SPEECH chunk=" + event.index + " last=" + event.last)
            output.playNext()
        }
    }
    MediaPlayer {
        id: player
        objectName: "speechPlayer"
        audioOutput: AudioOutput {}
        onMediaStatusChanged: if (mediaStatus === MediaPlayer.EndOfMedia && output.neuralActive) output.playNext()
        onErrorOccurred: (error, message) => { if (output.neuralActive) output.fallBack(message) }
    }
    TextToSpeech {
        id: system
        objectName: "systemSpeech"
        onStateChanged: state => { if (output.trace) console.warn("GAMEBUDDY_SPEECH_STATE " + state) }
    }
}
