# Subscription providers

GameBuddy can use the installed, signed-in **Codex CLI** and **Claude Code CLI**. It does not extract credentials from either desktop application. Model calls count against the signed-in account's applicable usage limits.

In **Options → Models**, choose **Use Codex subscription + live voice** or **Use Claude subscription + Codex voice**. These preserve the independent local vision configuration. An empty Codex model uses the CLI's configured model; the Claude preset uses `sonnet`. The installed Claude default may require usage credits, even when another model is available through the subscription.

```json
{
  "reasoning": {"kind":"codex", "endpoint":"", "model":"", "api_key_env":""},
  "vision": {"kind":"ollama", "endpoint":"http://127.0.0.1:11434", "model":"qwen3.5:4b", "api_key_env":""},
  "speech": "codex_realtime"
}
```

Use `claude_code` for Claude reasoning. `speech: "local"` selects the existing Whisper/Qt speech path. API-key providers remain separate settings. Credentials are owned and refreshed by the official CLIs; the subscription adapters remove API-key environment variables and never silently fall back to paid API requests.

## Realtime voice

The Codex adapter uses its experimental app-server **WebRTC v3** interface. Signaling travels over a private stdio connection; audio travels directly over WebRTC. The WebSocket transport currently requires API-key authentication and is deliberately not used. Claude's CLI supplies reasoning and image interpretation, not an embedded Claude Desktop speech service. Choosing Claude reasoning therefore still uses Codex for the voice session and its native tool handoffs.

During a launched game, **Hands-free** starts listening automatically if the microphone is already unmuted: just speak without clicking. A muted microphone waits for unmute. Live voice also connects for spoken guidance when listening is off and spoken replies are enabled, using a silent generated track without opening the microphone. GameBuddy introduces itself once per game session. Stable, source-backed changes to the suggested action trigger short spoken cues after a 12-second stability period, at most once every 90 seconds and once per action per game session. Unknown or unchanged locations remain quiet. Muting closes every microphone track while leaving output available. **Stop listening** stays stopped until Listen or another mute/unmute gesture; disabling **Hands-free** also disables activation on unmute. **Voice off** disables output and **Stop** pauses the voice session until Listen or Voice on resumes it. `--quiet` disables output; `--no-listen` disables Hands-free at launch. Voice remains independent of the overlay's keyboard focus. Only an explicit overlay voice command opens/focuses controls. GameBuddy tools expose guide context, reference excerpts, outline edits and widgets; they do not confirm objective completion or execute operating-system commands.

The voice web page is bundled with the application and served on a private, ephemeral localhost port. Only that exact origin receives microphone permission while microphone activation is on. The frameless transport requires an incoming audio clock for proactive output: a locally generated zero-valued Web Audio track supplies it while capture is off, without accessing an input device. Speech cues use the installed app-server’s `thread/realtime/appendSpeech`. The latest visual context is sent on player speech using `session.context.append` with `channel: commentary` over the same WebRTC data channel, in Unicode-safe chunks of at most 500 bytes. App-server `appendText` currently defaults to speakable content even with a developer role, so it must not carry periodic observations. Unchanged context is suppressed; idle perception refreshes do not send voice messages. A selected cue carries its own spoken wording; sending commentary beside it can cancel queued speech. A local playback gate requires a detected player turn or explicit cue and follows native turn completion instead of a fixed speech-duration budget. The echo-cancelled microphone remains live for interruptions and immediate replies. See [voice input](voice-input.md) for completion, timeout and interruption behavior. An unsupported CLI, lost connection or rejected account is reported in the companion; it does not silently enable robotic speech or switch billing providers.

The selected Codex reasoning model is also used for native voice tool handoffs. **Faster Codex voice replies** is enabled by default (`fast_voice: true`): the adapter requests the priority service tier only when the CLI advertises it for that model. Fast mode consumes subscription quota faster; turn it off in Models for ordinary service. The setting applies on the next voice connection and never substitutes a different model. Voice reasoning uses low reasoning effort, concise answers and focused source-backed context. `get_game_context` includes nearby eligible actions; `search_guide` and selected `get_outline.step_ids` avoid loading a large complete walkthrough for ordinary advice. Full outlines and history remain available when needed.

The speaker and reasoning backend have separate instructions. Each delegated player turn receives current context and a native screenshot after the utterance ends, with ordinary context tools retained as a fallback. The backend inspects images, completes widget changes and publishes next-action advice before the speaker confirms them. Streamed user fragments are retained until the utterance ends so a later clause is not lost when the native speaker delegates early. Pending mutating calls planned during speech are rejected with the complete request for reconsideration. Progress, source and revision checks remain authoritative.

## Conversation continuity

GameBuddy stores completed user and approved assistant voice transcripts immediately in
its local `gamebuddy.sqlite`, alongside text-coach exchanges. History belongs
to the game title (case and whitespace normalized), independently of the selected
guide, guide revision, provider, voice connection, or capture session. Returning
players receive a welcome-back greeting. A new voice session receives the recent
dialogue as quoted developer context, not user/assistant turns to replay. The
reasoning handoff can retrieve the archive using `get_conversation`; the text
coach receives recent history directly. The `get_conversation` tool searches and pages older exchanges.

Startup history is bounded to 24 messages and 4 KB of serialized text, with
explicitly marked excerpts for long messages. The full transcript remains in the
database; this is retrieval-based memory, not an unlimited model context or an
automatic summary. Existing guide-scoped text history is imported once. Voice
exchanges made before this feature were not saved and cannot be recovered.

Conversation recall does not restore emulator state or replay past location and
completion reports. Historical dialogue is identified as such; current screen
observations and new reports determine present location. Loading a save state is
not currently an explicit emulator event received by GameBuddy. Assistant text
records generated speech, which could have been muted or interrupted, and an
utterance cut off before a final transcript arrives is not guaranteed to be saved.
Raw microphone audio is not added to the conversation archive. Recent history and
requested search results are sent to the selected conversation provider as context.

## Walkthrough preparation and screen awareness

Preparation downloads/cache-reuses video and available captions, builds a visual index, divides uncompiled chapters into two-minute intervals, and asks the selected reasoning provider to interpret twelve ordered frames plus timed transcript evidence per interval. If captions are missing, local whisper.cpp transcribes that interval. Each successful section is saved immediately; failed sections can be retried on the next preparation. Player-created steps and existing compiled children are preserved.

The built-in HTML outline is always available, supports nested collapse/expand and opens the suggested action's ancestors. Conversation uses `set_next_action` to share the latest eligible action and instruction with both the outline and Now card; this does not report location or complete a task. Clicking a step only inspects it; it does not confirm progress. The preparation indicator distinguishes pending sections and failed reviews. Reviewing every interval is not proof of a complete or correct walkthrough: sparse frame sampling and transcription can miss events.

Game-only capture runs every second, independently of conversation and reference preparation. The configured vision provider starts interpreting changed frames no more often than every three seconds; identical images may be rechecked after four seconds. One semantic observation runs at a time, so model latency also limits the cadence. Capture and geometric tracking continue independently; prepared geometric references disable legacy automatic candidate comparison. The default uses local Ollama; selecting a remote vision provider sends these snapshots to that provider. Observations describe visible objects, readable text and significant events, with uncertainty. The latest eligible summary enters voice context on player speech; gameplay questions also request the current image. New eligible actions speak their instructions on the next narration poll, with pending instructions retained during conversations. A separate background review compares up to thirty seconds of screenshots with current and locally relevant guide actions and their parent strategy notes. At most one review runs at a time, starting no more often than every eight seconds. It can publish a fresh next action or a short source-backed strategy for a changed situation or visible difficulty; this does not itself complete an action. Requested replies and automatic coaching use the same approved speech path. See [voice input](voice-input.md) for timing and freshness rules.

`get_game_context` and `get_game_screen` supply a fresh actual JPEG through native image content to the Codex reasoning handoff and MCP clients. The ordinary pluggable coach also attaches it to reasoning requests. Captures do not require a walkthrough or visual index. Frames expire after fifteen seconds; analysis expires after twelve, or earlier when a visual signature differs from the latest frame. A late result for a visibly changed frame is discarded. The signature is a coarse check and cannot establish that every small change was observed. **Pause watching** immediately revokes tool image access and discards pending results; **Resume watching** starts with a fresh frame. The companion displays capture/analysis status separately from location confidence. Ollama observations use a required JSON schema, including confidence, player position, obstacles, threats and exits. The small observer remains fallible: gameplay reasoning receives the native screenshot and map pixels and must assess them itself. Only the latest frame is retained in the screen record, not a gameplay recording. Abrupt termination leaves a record that becomes stale and inaccessible to the tool.

**Crop…** beside that status, or **Crop game view** in Options, opens a frozen full-frame preview. Draw a rectangle around gameplay, drag its interior to move it, or drag a corner to resize it. The right-hand preview shows the selected region. **Save crop** remembers normalized coordinates per game, before image downsampling, and applies them to local observation, voice/MCP screenshots, reference retrieval and candidate-image comparison. Images are bounded to 1280×960 without upscaling small crops. Include the HUD if the model should read it. RetroArch's display settings and walkthrough-video crops are separate. Reselect the region when changing the bezel/layout or aspect ratio.

Saving clears previous scene/location estimates and rejects pending analysis from the old crop; it retains confirmed progress. The next capture supplies a fresh image. **Reset to full frame** changes the draft; save it to restore full-frame analysis. **Cancel** and Escape discard the draft. A crop improves the input but does not guarantee correct building names or room identification.

The prepared geometric tracker owns location and map footprints; see [visual localization](visual-localization.md). When those references are unavailable, legacy fingerprint retrieval separately matches reference frames. Repeated, sufficiently distinct matches select a location estimate and update the next-action widget without a question from the player. When retrieval is ambiguous, the local Ollama VLM compares the live frame with likely references, at most every 15 seconds; two agreeing, recent observations are required to select a VLM location estimate. Stale results after capture pause, guide revision or progress reset are discarded. Remote candidate comparison still uses the explicit Compare control. This is periodic visual observation, not full-motion understanding or frame-perfect danger detection. Independent-gameplay accuracy, especially with shaders, HUD differences and similar rooms, still needs evaluation. No visual match automatically marks an objective complete.

## Development and checks

`./dev.sh` enters the GameBuddy Nix shell and runs an initial build, then rebuilds on Rust, C++, QML, HTML and Cargo changes using watchexec. `./dev.sh --once` builds once. It does not restart a live game. Lunchpail's launch wrapper continues to execute the last built binary without invoking Cargo.

- `cargo run -p gamebuddy-services --example subscription_probe -- codex [image]`
- `GAMEBUDDY_PROBE_MODEL=sonnet cargo run -p gamebuddy-services --example subscription_probe -- claude [image]`
- `scripts/test-realtime` uses a subscription call and isolated virtual audio devices to check the real microphone-to-WebRTC-to-widget path, including output looped back into the test microphone and silent intervals after the greeting and reply.
- `scripts/test-conversation` launches the real app twice with private virtual audio: the first session learns a phrase, and the second recalls it into a widget without the phrase in its request.
- `scripts/test-gamescope` verifies the compositor and HTML widget interactions.

Protocol reference: [Codex app-server](https://developers.openai.com/codex/app-server), version-matched schemas from `codex app-server generate-json-schema --experimental`. Subscription policy: [Claude Agent SDK and Claude plans](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan). These interfaces and account policies can change.
