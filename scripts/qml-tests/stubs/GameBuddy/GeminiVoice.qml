import QtQuick

// Test double for the Rust GeminiVoice used by the settings page.
QtObject {
    property string savedKey: ""
    function save_api_key(key) { if (!key.trim().length) return "Enter a Gemini API key"; savedKey = key.trim(); return "" }
    function has_api_key(directory) { return savedKey.length > 0 }
}
