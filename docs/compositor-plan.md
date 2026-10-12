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
   *Done October 12:* overlays keep `STEAM_OVERLAY`/`STEAM_INPUT_FOCUS`, stay above the
   game with working transparency, and receive keyboard/pointer per mode. Client cursors
   are drawn by the compositor and `GAMESCOPE_CURSOR_VISIBLE_FEEDBACK` follows them. For
   the existing probes it also honours Gamescope's `GAMESCOPE_XWAYLAND_SERVER_ID` and
   debug screenshot request, and accepts Gamescope's command line, so
   `GAMEBUDDY_GAMESCOPE_BIN=target/debug/gamebuddy-compositor scripts/test-gamescope`
   runs the real overlay against it. It passes the same steps as Gamescope 3.16.29 in a
   headless sway session; both stop at the same pre-existing tray-restore check.
   Smithay's window manager reparents clients into frames, so GameBuddy's X11 code now
   looks inside frames (`client_windows`), and Smithay is vendored with a patch that gives
   frames the client's visual (otherwise ARGB overlays lose transparency).
3. **Input fidelity.** value120 scroll, pinch, relative pointer, pointer constraints.
   This is what the Gamescope patch added.
   *Implemented October 12:* winit cannot deliver touchpad gestures, relative motion or
   pointer locks, so on Wayland hosts the compositor is now a native Wayland client
   (`host.rs`, smithay-client-toolkit) with its own EGL surface, fractional scaling and
   the host keymap. It forwards value120/axis-source/stop scrolling, pinch, swipe and
   hold gestures, and relative motion, and offers `zwp_pointer_gestures_v1`,
   `zwp_relative_pointer_v1` and `zwp_pointer_constraints_v1` to Xwayland, mirroring a
   game's pointer lock on the host. winit remains the fallback on X11 hosts. The overlay
   probe gives the same result as on Gamescope; gestures and pointer lock still need a
   check on a real desktop with a touchpad, which the headless test session cannot do.
4. **Capture.** Read the game surface's buffer directly from the renderer, replacing
   XComposite readback.
5. **Switch.** `--compositor builtin|gamescope` flag, run the existing compositor,
   resolution and input probes against both, Faxanadu end to end through Lunchpail, then
   make builtin the default and remove Gamescope and its patch from `flake.nix`.
   *Done October 12 (ahead of milestone 4):* `gamebuddy --launch`/`--gamescope-bridge`
   now start the compositor from the GameBuddy executable (`--nested-compositor`).
   Gamescope, its patch and `GAMEBUDDY_GAMESCOPE_BIN` are gone from the flake. The
   overlay probe through the real launch path matches the Gamescope baseline. Game
   capture still uses XComposite through the frame; milestone 4 remains an optimization.
