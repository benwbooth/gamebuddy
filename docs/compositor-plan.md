# GameBuddy compositor plan

October 12, 2026. GameBuddy replaces its nested Gamescope session with its own
compositor, using [Gamescope 3.16.29](https://github.com/ValveSoftware/gamescope/tree/3.16.29)
as the behavioral reference. The patched Gamescope is not in any binary cache, so every
Nix garbage collection forced a local rebuild at game launch, and every input fix meant
carrying a C++ patch (`nix/gamescope-input.patch`).

## Approach

A new workspace crate, `gamebuddy-compositor`, built on
[Smithay](https://github.com/Smithay/smithay) (Rust, MIT). Smithay supplies the Wayland
protocol state, the nested winit/Wayland/X11 host backends, a GLES renderer and an
`XWayland`/`X11Wm` implementation. GameBuddy owns composition, focus and input policy.

The compositor runs in-process with the supervisor, so capture and overlay state become
Rust calls instead of X11 atoms and screenshot files.

## Contract to preserve

These are what the current Gamescope adapter provides (`crates/gamebuddy-overlay`,
[overlay backends](overlay-backends.md), [session protocol](gamescope-session-protocol.md)).
Lunchpail must not need changes until the final switch.

1. A nested fullscreen window on the host Wayland or X11 desktop, at the host output's
   physical mode, with nearest filtering.
2. One private Xwayland server. `ready.json` publishes its `DISPLAY`. The game and the Qt
   overlay are both X11 clients on it.
3. The game is composed fullscreen beneath the overlay. Overlay windows are identified as
   today (the `STEAM_OVERLAY` property) and drawn on top with alpha.
4. Focus states (hidden, passive, interactive) and `STEAM_INPUT_FOCUS`-equivalent routing:
   pointer goes to the overlay inside its reported input regions, otherwise to the game.
   Grabs stay inside the private session.
5. Full input fidelity from the host: high-resolution (value120) scrolling, axis sources,
   pinch gestures, relative pointer and pointer locking for games that capture the mouse.
6. Game-only capture, excluding GameBuddy surfaces.
7. Session lifecycle: stop marker, parent-death teardown, game exit ends the session,
   overlay crash leaves the game running.

## Milestones

1. **Nested Xwayland host.** Winit window on the host, Xwayland started, X11 windows
   mapped and composed (game fullscreen, others stacked), `DISPLAY` reported. Verified
   with an X11 fixture and with RetroArch running Faxanadu.
   *Done October 12:* `target/debug/gamebuddy-compositor [--fullscreen] [--size WxH]
   [--ready-file PATH] [-- COMMAND]`. Verified inside a headless sway session: glxgears
   at 62 fps through dmabuf, and Flatpak RetroArch (FCEUmm) showing Faxanadu's title
   screen at 1280×720.
2. **Overlay layering and focus.** Overlay classification, input regions, the three
   focus states, keyboard/pointer routing, control-socket actions.
3. **Input fidelity.** value120 scroll, pinch, relative pointer, pointer constraints.
   This is what the Gamescope patch added.
4. **Capture.** Read the game surface's buffer directly from the renderer, replacing
   XComposite readback.
5. **Switch.** `--compositor builtin|gamescope` flag, run the existing compositor,
   resolution and input probes against both, Faxanadu end to end through Lunchpail, then
   make builtin the default and remove Gamescope and its patch from `flake.nix`.
