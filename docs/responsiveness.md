# Responsive guidance and speech

GameBuddy now separates work that can be selected from current state from work that needs a model. The same saved progress, outcome rules and navigation recommendation feed the Now card, arrow and spoken instruction.

## Local responses and preparation

Complete routine questions such as “What should I do next?” select the current source-backed instruction locally. “Repeat that” uses this path only when the preceding reply was that still-current instruction. Unknown location stays explicit. A why-question uses a locally available bypass explanation when one exists; other explanations use reasoning. Compound questions, arrival reports, route edits and widget requests retain their complete request and tools.

Navigation prepares a bounded frontier: the current instruction, nearby next instructions, the result of applying a current action's known effects to a **copy** of progress, and short urgent cues. Hypothetical progress is never saved. At most eight phrases are prepared, split into at most 32 background sentences. Preparation grants no permission to speak. Each actual request rechecks its guide revision, progress epoch and, for visual advice, current capture. A bounded asynchronous check also revokes stale advice during queued or active playback.

Optional Piper speech runs on the CPU in a persistent worker with two inference threads. A foreground request replaces queued old speech and takes priority over background preparation. Sentences play as they become ready, in order; later cached sentences cannot overtake an earlier uncached one. Captions show the sentence whose audio has started. Native realtime speech retains its word timestamps. Interruption, mute, disconnection and expired advice discard pending audio and captions together. The live WebRTC microphone remains available during either speech path.

Install and enable prepared speech after building the CLI:

```sh
scripts/setup-prepared-speech
# A separate profile can be configured without changing the default profile:
scripts/setup-prepared-speech --data-dir /path/to/profile
```

The helper uses the project's pinned Nix runtime, or `uv` and Piper 1.8.0 without Nix. It downloads a checksum-verified LJSpeech medium voice and its model card. Its training dataset is public domain; the downloaded model card is retained beside the model. Existing provider and planning settings are preserved. **Options → Models → Prepared neural speech · local voice** enables/disables it after installation; reconnect voice for the setting to take effect. The voice sounds different from Codex's native voice. A failed initial synthesis falls back to native speech; failure after playback starts leaves the complete answer visible without repeating its already-spoken beginning.

## One model decision instead of a tool round trip

Short eligible questions can use a read-only Codex proposal with compact current evidence and the actual screenshot. Its constrained result contains only an answer, an optional eligible action ID and an optional supported screen direction. Local validation publishes the same instruction that will be spoken. It cannot complete steps, change inventory or edit goals.

The listener and tool reasoner use separate provider conversations. Only the application's settled request enters the tool conversation; merged native transcripts and unsolicited listener delegations cannot replay previous tool actions.

Stable partial recognition may start a proposal while the player is speaking. There are at most two attempts per utterance, at least one second apart. Changed wording cancels the candidate. Only an exact match to the final settled request can be applied; endpointing is not shortened. Changed captures, revisions or progress reject proposals. Requests requiring other tools go to the general reasoner. Proposals have a twelve-second deadline and a single replaceable pending job. Background action review yields new work while a foreground voice request is active.

## Fast screen events

Game capture polls every 100 ms with one capture in flight. A separate bounded local detector uses Tesseract, one inference thread and a 700 ms process deadline. It recognizes high-confidence complete event lines such as “GAME OVER”, “YOU DIED” and “VICTORY”, and explicit numeric `HP: current / maximum` displays at or below 20%. Two distinct captures must agree. Identical images reuse their OCR result. Events expire after 1.5 seconds and lose permission when the capture changes or pauses.

Death and low-health cues can interrupt ordinary advice or pending reasoning once the player stops speaking. The interrupted answer and its pending tools are cancelled. Repeated model-derived danger/death observations remain a fallback, with a stricter three-second freshness limit. These events do not mark game objectives complete. Pixel fonts, stylized meters and unlabeled icons are not covered by the generic text detector. The optional ROM-specific emulator adapter now polls independently of visual-model startup; its existing revision/process checks remain required, and unverified RAM semantics do not become inventory facts.

## Measurements and reproducible checks

Measurements use the production Qt/Gamescope application, subscription recognition, a private virtual microphone, an echo-looped output sink and recorded output. They are development-machine measurements, not universal latency guarantees. “Playback” below means the output meter detected actual speech; recorded audio was also transcribed independently.

| Check | Result on 2026-10-05 |
| --- | --- |
| Eight routine “What should I do next?” requests | 1.253–1.475 s from the request WAV ending to speech; median 1.403 s; no full reasoning calls or progress mutations |
| Changed screen during a playing instruction | Playback permission revoked in 263 ms; microphone stayed open |
| Readable Game Over during a pending answer | Warnings began in 539 ms and 641 ms across two covered-text runs; obsolete answers remained cancelled |
| Retro block-font Game Over fixture | Not recognized confidently by OCR; the vision fallback took about 6.2 s |
| Five source-backed Mattock-purpose proposals | 2.948–4.427 s for the model decision, before voice endpointing/playback |
| Forced 35-second answer hold | Recorded silence while waiting, with no spoken filler; microphone remained usable |
| Long reply and barge-in | A 114.0-second reply reached its independently transcribed final words; a spoken stop returned to waiting in 1.2 s |

The repeated routine run includes sentence streaming and separate listener/reasoner conversations. Its maximum is below 1.5 seconds, but eight samples do not establish a population P95, and the proposed one-second median is not met. Native recognition sometimes provides a late partial instead of a final transcript; the existing 800 ms stability safeguard remains to preserve later clauses. General multi-tool or long-form requests still take seconds, sometimes substantially longer.

A direct realtime WebSocket comparison was attempted with the installed Codex app-server schema. That transport rejected subscription authentication with `realtime conversation requires API key auth`; no API key was configured, so it did not produce a latency measurement. The existing subscription WebRTC transport works. This failure does not establish that all direct speech architectures are unavailable, and the app does not silently switch to paid API credentials.

```sh
# Run inside nix develop. Prepared settings affect only disposable test stores.
export GAMEBUDDY_TEST_PREPARED_PYTHON="$HOME/.local/share/GameBuddy/models/piper/runtime/bin/python"
export GAMEBUDDY_TEST_PREPARED_MODEL="$HOME/.local/share/GameBuddy/models/piper/en_US-ljspeech-medium.onnx"
export GAMEBUDDY_TEST_VOICE_MODEL=gpt-6.1-sol
GAMEBUDDY_TEST_LOCATION=1 GAMEBUDDY_TEST_ROUTINE=1 scripts/test-realtime
GAMEBUDDY_TEST_RESPONSIVE=1 scripts/test-realtime
GAMEBUDDY_TEST_WORKING=1 scripts/test-realtime
# Supply a readable event image to test the covered OCR path:
GAMEBUDDY_TEST_RESPONSIVE=1 GAMEBUDDY_TEST_FAST_EVENT=1 \
  GAMEBUDDY_TEST_EVENT_IMAGE=/path/to/game-over.png scripts/test-realtime
node scripts/test-voice-lifecycle.mjs
```

`decision_probe` benchmarks the one-shot model against a private `/tmp` store and actual supplied image, without applying its proposal. `fast_perception_probe` measures the real local detector. Deterministic tests cover partial-utterance cancellation, stale capture/progress rejection, bounded speech ordering, late download cancellation, muting and urgent-event priority. Physical microphone acoustics and representative gameplay still need hands-on testing.
