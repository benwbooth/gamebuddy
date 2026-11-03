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
    // The sentence being heard: its words and when its sound starts and
    // ends, reported by the voice as it plays.
    property string sentence: ""
    property double sentenceStart: 0
    property double sentenceEnd: 0
    // How much of `text` has been heard, for the captions; -1 when this
    // voice does not report its timing (the whole text is shown).
    property int spokenChars: -1
    // Set once the voice has reported a sentence start: it streams, so a
    // caption waits for the sound instead of showing the whole reply first.
    property bool reportsTiming: false
    Timer {
        interval: 100; repeat: true; running: output.speaking
        onTriggered: {
            const at = output.position(Date.now())
            output.spokenChars = at >= 0 ? at : (output.reportsTiming ? 0 : -1)
        }
    }
    // How far into `text` the voice is at `now`, in characters; -1 when not
    // known.
    function position(now) {
        if (!sentence.length || !sentenceStart || now < sentenceStart) return reportsTiming ? Math.max(0, text.indexOf(sentence)) : -1
        const offset = text.indexOf(sentence)
        if (offset < 0) return -1
        const fraction = sentenceEnd > sentenceStart
            ? (now - sentenceStart) / (sentenceEnd - sentenceStart)
            : (now - sentenceStart) / 1000 * 15 / sentence.length
        return offset + Math.round(Math.max(0, Math.min(1, fraction)) * sentence.length)
    }
    readonly property bool neuralAvailable: neural.available
    readonly property bool speaking: neuralActive || system.state === TextToSpeech.Speaking
    readonly property bool busy: neuralActive || (system.state !== TextToSpeech.Ready && system.state !== TextToSpeech.Error)

    onDirectoryChanged: if (directory.length) neural.configure(directory)

    // Sentences waiting for the one being spoken to finish.
    property var queued: []
    // Speak after whatever is being said now; a streamed answer arrives one
    // sentence at a time. A filler ("One sec.") is cut off, not waited for.
    function append(value) {
        if (!speaking || filler) { filler = false; say(value, true); return }
        queued = queued.concat([value])
    }
    property bool filler: false
    function sayNext() {
        if (speaking || !queued.length) return
        const next = queued[0]
        queued = queued.slice(1)
        say(next, true)
    }
    onSpeakingChanged: if (!speaking) Qt.callLater(sayNext)
    function say(value, keepQueue) {
        if (!keepQueue) queued = []
        stop()
        text = value
        if (directory.length && neural.configure(directory) && neural.speak(value)) {
            neuralActive = true
            return
        }
        system.say(value)
    }
    // Stop and forget anything queued (the player interrupted or asked to stop).
    function silence() {
        queued = []; filler = false
        stop()
    }
    function stop() {
        neural.stop()
        neuralActive = false
        streamEnd.stop()
        sentence = ""; sentenceStart = 0; sentenceEnd = 0; spokenChars = -1
        chunks = ({}); nextIndex = 0; lastIndex = -1; spoken = ""; lastText = ""; loaded = -1; current = 0
        player.stop()
        spare.stop()
        system.stop()
    }
    // Streamed speech arrives in short pieces. Two players take turns, the
    // idle one loaded with the next piece, so one starts the moment the other
    // ends; a single player reloading between pieces made the voice stutter.
    property int current: 0
    property int loaded: -1
    function playing(p) { return p.playbackState === MediaPlayer.PlayingState }
    function playNext() {
        const now = current === 0 ? player : spare
        const next = current === 0 ? spare : player
        if (playing(now)) {
            const chunk = chunks[nextIndex]
            if (chunk && loaded !== nextIndex) { next.source = chunk.url; loaded = nextIndex }
            return
        }
        const chunk = chunks[nextIndex]
        if (!chunk) {
            // Sentences can finish synthesizing out of order.
            if (lastIndex >= 0 && nextIndex > lastIndex) neuralActive = false
            return
        }
        const start = loaded === nextIndex ? next : now
        if (start === now) now.source = chunk.url
        current = start === player ? 0 : 1
        loaded = -1
        nextIndex++
        if (chunk.text !== lastText) spoken += (spoken.length ? " " : "") + chunk.text
        lastText = chunk.text
        start.play()
        const following = chunks[nextIndex]
        if (following) {
            (start === player ? spare : player).source = following.url
            loaded = nextIndex
        }
    }
    property string lastText: ""
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
            // Already playing on the voice's own stream: show it as speaking
            // until its sound ends.
            if (event.kind === "started") {
                output.reportsTiming = true
                output.sentence = event.text
                output.sentenceStart = event.at_ms
                output.sentenceEnd = 0
                return
            }
            if (event.kind === "played") {
                if (event.text === output.sentence) output.sentenceEnd = event.until_ms
                output.spoken += (output.spoken.length ? " " : "") + event.text
                if (event.last) {
                    streamEnd.interval = Math.max(1, event.until_ms - Date.now() + 150)
                    streamEnd.restart()
                }
                return
            }
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
    Timer { id: streamEnd; repeat: false; onTriggered: output.neuralActive = false }
    MediaPlayer {
        id: spare
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
