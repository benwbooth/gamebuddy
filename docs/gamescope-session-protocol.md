# Gamescope session protocol v1

This is a local Linux launch contract between independent GameBuddy and Lunchpail projects. No link-time dependency is required.

1. Launcher creates a mode-0700 temporary directory.
2. Spawn `gamebuddy --gamescope-bridge DIRECTORY --game TITLE --game-id ID --platform PLATFORM --session-id TOKEN`. Use literal argv and preserve the user's game launch working directory.
3. Wait for `ready.json`, or for the supervisor to exit. Cancel or time out by writing `stop` in the directory. `ready.json` is atomically renamed into place and contains `{"version":1,"display":":N"}`. Reject unknown versions and non-local display names.
4. Spawn the actual game normally, with `DISPLAY` set to the returned display, `WAYLAND_DISPLAY` cleared, and X11 selected for toolkits (`SDL_VIDEODRIVER=x11`, `QT_QPA_PLATFORM=xcb`, `GDK_BACKEND=x11`). A `flatpak run` launch also needs `--socket=x11 --nosocket=wayland` before the application ID. Keep the original executable, cwd, child PID and control pipes.
5. After startup succeeds, atomically write `game.json` as `{"game_pid":12345}`. This starts the overlay UI and enables process-scoped frame selection.
6. GameBuddy's UI binds `DIRECTORY/control.sock`. Send one UTF-8 datagram containing `toggle`, `toggle-visibility`, `show`, `hide`, `focus`, or `release`. `toggle` opens/focuses or releases the companion. `show` makes passive surfaces visible. `hide` hides everything and releases focus. `focus` shows and opens the companion. `release` ends interaction without changing visibility. The socket may not exist immediately after game attachment; report/retry a binding action when the UI is ready. Delivery does not currently return an acknowledgement.
7. When the game exits or launch is cancelled, write `stop`, wait for the supervisor, and remove the directory. GameBuddy also monitors the attached process and tears down when it disappears. Do not remove the directory while continuing to use its compositor.

The GameBuddy supervisor owns Gamescope and the UI. Lunchpail owns the emulator. GameBuddy does not proxy emulator stdin/stdout, so calibrated controller transport remains direct. Compositor preparation has a bounded wait and must observe launch cancellation. An overlay-only crash does not deliberately terminate the game.

Keyboard bindings are session-local defaults: Shift+Tab for interaction, Ctrl+Shift+Tab for visibility, Escape for release. Physical gamepad mapping and acknowledgement/status feedback are future protocol consumers/extensions.
