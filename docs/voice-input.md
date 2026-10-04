# Microphone activation

GameBuddy owns microphone activation and overlay focus. Lunchpail supplies the game launch context and emulator configuration.

**Hands-free** is enabled by default when launching a game with GameBuddy, including through Lunchpail. An already-unmuted microphone starts listening automatically: just speak, without clicking the overlay or toggling mute first. A muted microphone stays closed until you unmute it. **Stop listening** closes capture and stays stopped; a later system mute/unmute gesture or **Listen** can resume it. Disable **Hands-free** to use only Listen or hold-to-talk. Enabling it again starts listening if the selected microphone is unmuted. Opening the standalone companion/workbench does not automatically listen; `--no-listen` also starts a launched game with Hands-free disabled.

With Codex live voice configured, a launched game automatically connects for a short introduction and occasional next-action cues. With the microphone muted or listening disabled, output-only mode sends a generated silent track to keep the realtime transport clock advancing without opening a microphone. Muting stops capture but leaves guide speech available. **Voice off** disables output, **Stop** pauses the voice session, and `--quiet` starts with output disabled (listening is controlled separately). The local Whisper/Qt backend also starts listening automatically and retains its existing voice quality.

Automatic hints are at least 90 seconds apart, require a stable action for 12 seconds, and mention each action at most once per game session. Routine menus, dialogue and area changes stay silent. Only repeated confident death, danger or victory observations can produce an event cue, with a five-minute cooldown per event type. Automatic speech pauses during conversation and for 45 seconds afterward; the task visible during conversation is not repeated as a delayed hint. These limits do not delay requested replies. Both reasoning paths are instructed to answer in one or two short sentences unless asked for detail, placing longer instructions in widgets. Refreshing the OSD never generates a spoken answer by itself.

Live voice playback also requires a locally detected player turn or an explicit application cue. It returns to waiting after 2.5 seconds without reply audio, allowing a pending reasoning/tool handoff to finish. Continuous replies have a 20-second audible budget; automatic cues have 12 seconds. Unused permissions expire. Background perception and widget updates continue, but voice receives their newest context on a player turn or selected cue instead of every refresh. Saved dialogue is quoted historical context, not live user/assistant turns to continue.

To prevent speaker echo from becoming a new request, live voice sends silence from the selected microphone during audible assistant playback and for 800 ms afterward. The device remains open and listening resumes automatically; mute and Stop listening still close it. This prevents talking over the assistant from starting a new turn during that interval. **Stop** remains available to stop speech.

The collapsed icon turns red and animates while capturing, including WebRTC capture. An amber dot indicates an active conversation paused for processing or playback. Starting a conversation restores a hidden icon but does not expand the overlay or change game input focus. Muting closes capture and submits any sufficiently long speech already buffered; silence and short transients are discarded. Existing widgets remain visible. Device changes or loss of mute-state visibility stop automatic capture.

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

## Automatic game volume

**Lower game volume during conversations** is enabled by default in Options → Models (`duck_game_audio` in settings). A launched game's audio fades down by **12 dB** when microphone speech is detected, while a local voice request is processed, while hold-to-talk records, or when GameBuddy speaks. An unmuted but silent microphone and an idle voice connection do not lower it. The game returns to its normal level after 1.8 seconds without conversation activity, with a short release fade. Realtime voice holds through a pending reply for up to twenty seconds; a later audible reply ducks again automatically.

For realtime input, the existing WebRTC/libfvad detector receives ephemeral 20 ms, 16 kHz samples from the already-authorized Chromium microphone stream. No second microphone is opened; Qt consumes the activity messages locally without writing audio files or application traces. Output activity comes from the actual received audio, respecting Voice off, rather than the presence of an audio track. Local voice uses its existing VAD and Qt speech state. Closing a voice session clears activity.

The current volume backend requires **Linux with PipeWire**, including applications using its PulseAudio compatibility server. It follows the launch PID and descendants, including uniquely identifiable container processes. It applies temporary per-stream software attenuation; it never changes system volume or the application's saved mixer level. New game streams are discovered during conversations. A manual mixer volume adjustment takes precedence for that stream until the conversation ends. Ambiguous process ownership is left untouched.

A separate Rust helper owns the attenuation and restores surviving streams when the overlay closes or crashes. A three-second heartbeat timeout also releases it if the UI stops responding. Disabling the option releases the helper immediately. The helper uses `pw-dump` and `pw-cli`, already present in the Nix runtime. Windows/macOS and a standalone PulseAudio server do not yet have ducking adapters; the settings page reports their availability. Unrelated apps and GameBuddy output are outside the launched game's process tree.

`nix develop --command cargo run -p gamebuddy-services --example audio_duck_probe` checks actual output attenuation on a private virtual sink, restoration, channel balance, manual volume changes, recreated streams and helper EOF. `scripts/test-realtime` also checks ducking through the production WebRTC microphone and spoken introduction, including silence with an active microphone. Both fixtures avoid physical audio devices.

## Platform boundaries

Automatic system mute detection currently supports Linux PulseAudio and PipeWire's PulseAudio compatibility server. A Rust background worker queries the exact Qt-selected source through `pactl` every 250 ms, with a bounded timeout; it never changes the system mute state. The Nix shell includes `pactl`. A physical microphone's mute button must expose its state through that server to trigger activation. The Shure MV7+ was identified as the host's default input, but its physical mute button has not been tested.

The shared Qt capture path and Listen/hold-to-talk controls remain available to other platforms, but Windows/macOS system mute adapters and native runtime verification are pending. Qt microphone permissions and macOS packaging declarations still apply. `GAMEBUDDY_AUDIO_SOURCE` pins an explicit Qt device ID; disappearance never silently switches capture to another device. Without it, GameBuddy follows the default input and requires renewed activation after a device change.

## Verification

Run `nix develop --command scripts/test-voice` from the project. The test creates a temporary virtual sink/source, keeps the user's default input unchanged, feeds synthesized speech through Qt capture and the real local Whisper model, and launches the production overlay in a private Gamescope session. `scripts/test-gamescope` separately covers keyboard, icon and HTML widget interactions. Set `GAMEBUDDY_WHISPER_MODEL` to an existing model to avoid a fixture-local download.

The local test starts muted and checks mute/unmute capture lifecycle, passive focus, a spoken guide command, a question through a delayed local provider fixture, capture suspension during processing, resumption, and recording cleanup. `scripts/test-realtime` starts its virtual mic unmuted before launch, then sends a spoken request without clicking or toggling mute. It checks real subscription voice, screen tools, widgets, audio ducking, and capture closure on mute. Unit tests check that Stop, device changes, monitoring failures and capture failures do not trigger automatic restarts. Non-voice fixtures use `--no-listen`. The provider fixture checks integration, not model answer quality. `--voice-trace` logs capture and Qt speech-state transitions without transcript content. Physical microphone acoustics and hardware mute-button behavior need a separate hands-on check.
