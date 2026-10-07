import QtQuick

// Test double for the Rust NeuralVoice; tests deliver chunks via event_json.
QtObject {
    property bool available: true
    property string event_json: ""
    property var requests: []
    property int stops: 0
    function configure(directory) { return available }
    function speak(text) { if (!available) return false; requests = requests.concat([text]); return true }
    function stop() { stops++ }
}
