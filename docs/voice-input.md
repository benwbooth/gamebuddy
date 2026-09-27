# Microphone activation

GameBuddy owns microphone activation and overlay focus. Lunchpail supplies the game launch context and emulator configuration.

With **On unmute** enabled (the session default), muting then unmuting the selected system microphone starts a conversation. An already-unmuted microphone at startup does not automatically record. Choose **Listen** to start immediately in that case. **Stop listening** closes capture; a later system mute/unmute gesture can activate it again. Disable **On unmute** to use only the Listen button or the existing hold-to-talk controls.

The collapsed icon turns red and animates while capturing. An amber dot indicates an active conversation paused for processing or playback. Starting a conversation restores a hidden icon but does not expand the overlay or change game input focus. Muting closes capture and submits any sufficiently long speech already buffered; silence and short transients are discarded. Existing widgets remain visible. Device changes or loss of mute-state visibility stop automatic capture.

Speech is segmented locally using [WebRTC VAD through libfvad](https://github.com/kaegi/webrtc-vad), with a 200 ms pre-roll, at least 300 ms of detected speech, a 900 ms pause to end a turn, and a 25-second maximum turn. Audio comes from [Qt QAudioSource](https://doc.qt.io/qt-6/qaudiosource.html) through a small CXX bridge; activation, segmentation and WAV storage are Rust. The current capture format is 16 kHz mono PCM. A device that cannot provide this format reports an error and retains the hold-to-talk fallback.

Whisper transcribes each turn locally, then the configured reasoning provider receives the text and game context. Capture pauses while that voice turn is processed and while Qt speaks the reply, with an 800 ms recovery interval. This is turn-based conversation, not full-duplex interruption or acoustic echo cancellation. A headset helps keep game dialogue out of the microphone; VAD cannot distinguish the player from other audible speech.

Capture can continue during unrelated guide/perception jobs. One completed speech turn waits for the current job, then gets priority over the next automatic frame match. Audio is removed after transcription, including recognition/provider failures. A queued recording is discarded on orderly application shutdown. An abrupt process kill can leave a temporary WAV in the app's `voice` directory.

Explicit spoken commands bypass the reasoning provider:

| Command | Result |
| --- | --- |
| “Open the guide” | Opens the walkthrough and takes overlay focus |
| “Arrange widgets” | Opens interactive widget arrangement |
| “Back to the game” / “Close the guide” | Releases overlay interaction |
| “Hide the overlay” | Hides surfaces and releases focus |
| “Show the overlay” | Restores surfaces |

Ordinary strategy questions do not take focus. Shift+Tab, Ctrl+Shift+Tab and Escape remain available.

## Platform boundaries

Automatic system mute detection currently supports Linux PulseAudio and PipeWire's PulseAudio compatibility server. A Rust background worker queries the exact Qt-selected source through `pactl` every 250 ms, with a bounded timeout; it never changes the system mute state. The Nix shell includes `pactl`. A physical microphone's mute button must expose its state through that server to trigger activation. The Shure MV7+ was identified as the host's default input, but its physical mute button has not been tested.

The shared Qt capture path and Listen/hold-to-talk controls remain available to other platforms, but Windows/macOS system mute adapters and native runtime verification are pending. Qt microphone permissions and macOS packaging declarations still apply. `GAMEBUDDY_AUDIO_SOURCE` pins an explicit Qt device ID; disappearance never silently switches capture to another device. Without it, GameBuddy follows the default input and requires renewed activation after a device change.

## Verification

Run `nix develop --command scripts/test-voice` from the project. The test creates a temporary virtual sink/source, keeps the user's default input unchanged, feeds synthesized speech through Qt capture and the real local Whisper model, and launches the production overlay in a private Gamescope session. `scripts/test-gamescope` separately covers keyboard, icon and HTML widget interactions. Set `GAMEBUDDY_WHISPER_MODEL` to an existing model to avoid a fixture-local download.

The test checks mute/unmute capture lifecycle, passive focus, a spoken guide command, a question through a delayed local provider fixture, capture suspension during processing, resumption, and recording cleanup. The provider fixture checks integration, not model answer quality. `--voice-trace` logs capture and Qt speech-state transitions without transcript content. Physical microphone acoustics and hardware mute-button behavior need a separate hands-on check.
