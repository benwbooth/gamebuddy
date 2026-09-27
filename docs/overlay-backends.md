# Overlay backends

September 27, 2026. The Linux Gamescope backend is implemented. Windows and macOS currently retain the Qt desktop companion; graphics-hook backends remain separate work.

## Linux implementation

GameBuddy starts a nested Gamescope compositor with one private Xwayland server. The game and a single transparent Qt Quick/WebEngine canvas are composed there. The canvas contains the companion icon, HTML widgets and the optional workbench. No graphics library is injected into the game, and the host desktop is not reconfigured.

The development flake currently supplies **Gamescope 3.16.29**. The adapter uses `STEAM_OVERLAY` and `STEAM_INPUT_FOCUS` from [Gamescope's compositor](https://github.com/ValveSoftware/gamescope/blob/3.16.29/src/steamcompmgr.cpp). These are version-sensitive interfaces, not a stable public overlay SDK.

| State | Visible surfaces | Input |
| --- | --- | --- |
| Hidden | None | Game |
| Passive | Icon and enabled widgets | Game; hovering the icon gives GameBuddy pointer-only focus |
| Interactive | Expanded companion, editable widgets or options | GameBuddy keyboard/pointer focus |

Shift+Tab toggles interaction. Ctrl+Shift+Tab hides/restores all GameBuddy surfaces. Escape releases interaction. Grabs exist only on this session's Xwayland root, so they do not intercept typing in other desktop applications. The icon can also be clicked, dragged, or held for speech. Hidden game cursors do not activate hover focus; use the shortcut in games that lock/hide their pointer.

A private Unix datagram socket accepts `toggle`, `toggle-visibility`, `show`, `hide`, `focus` and `release`. The supervisor prints its path as `GAMEBUDDY_CONTROL_SOCKET`; in Lunchpail it is `control.sock` in the private session directory. GameBuddy's controller adapter should send these actions instead of synthesizing keyboard keys:

```sh
gamebuddy --overlay-action toggle /path/to/session/control.sock
```

Physical gamepad bindings are **not installed yet**. GameBuddy should own a configurable button/held chord and its device listener. Lunchpail supplies launch context and emulator configuration/arguments; it does not intercept keypresses. Preventing a chord from also reaching the game requires input routing in GameBuddy's compositor adapter; a device listener alone does not consume game input. Guide-button ownership, debounce, disconnects, and navigation inside the UI need controller-specific tests. Keyboard visibility/focus and the socket command path have runtime coverage.

### Wayland versus Xwayland

Gamescope itself is a Wayland compositor and can run nested on a Wayland desktop. This backend currently puts **both the game and the Qt overlay on its private Xwayland server**. That is a choice for the first interactive/capture adapter, not a limitation of Wayland as a display system.

Native Wayland game surfaces are not supported by this adapter yet. They need another capture path and verified focus/surface integration. Gamescope has native-client support and PipeWire capture facilities, but their existence alone does not prove a usable interactive GameBuddy backend. Keep native Wayland support as a separate adapter, rather than claiming that ordinary `xdg_toplevel` always-on-top windows solve the problem.

Flatpak launches explicitly receive `--socket=x11 --nosocket=wayland` for this session; otherwise `fallback-x11` can suppress the required X11 socket on a Wayland host. These are per-launch arguments, not persistent Flatpak permission changes.

### Capture

The synchronizer reads the game's XComposite named pixmap before overlay composition. Window selection requires the launched process or a descendant and excludes overlay-marked windows. XRes resolves host process IDs for Flatpak clients whose `_NET_WM_PID` is namespace-local. There is no desktop/root capture fallback.

Capture excludes **GameBuddy**; a game's own HUD or an emulator's built-in OSD is naturally part of its rendered frame. Current readback accepts 32-bit X image storage with the server's visual masks and byte order. Broader HDR, multi-window and native Wayland capture need further testing. Capture runs in the existing background job path; continuous perception while another model job runs remains separate work.

### Lunchpail lifecycle

Lunchpail asks `gamebuddy --gamescope-bridge PRIVATE_DIRECTORY` to prepare the compositor before spawning the emulator. GameBuddy publishes `ready.json` (`version: 1`, `display`). Lunchpail changes only the display environment (plus Flatpak socket arguments), then spawns the **original emulator executable** through its existing launch function. It keeps the real `Child`, process ID and controller stdin/stdout/stderr pipes. After successful startup, `game.json` supplies the actual game PID and GameBuddy starts the UI.

Dropping the launch session writes a stop marker and reaps the supervisor. GameBuddy-owned children have parent-death signals. Normal game exit tears down the overlay/compositor; losing the overlay alone leaves the game running. Standalone launches return the actual game exit code through a private result file. If compositor preparation fails, Lunchpail reports a warning and uses the desktop companion fallback.

## Other platforms

Valve documents that the [Steam overlay hooks graphics device creation](https://partner.steamgames.com/doc/features/overlay), supporting DirectX, OpenGL, Metal and Vulkan. Steamworks opens Steam's own UI; it is not an arbitrary GameBuddy HTML renderer.

| Platform | Next backend | Remaining boundary |
| --- | --- | --- |
| Windows | Thin graphics-hook DLL and out-of-process Rust host | Evaluate [hudhook](https://github.com/veeenu/hudhook) for D3D9/11/12 and OpenGL3; HTML texture/input bridge and Vulkan need work |
| macOS | Metal/OpenGL presentation adapter with native capture/input | Signing, hardened-runtime compatibility and native game tests |
| Linux native Wayland clients | Separate Gamescope adapter | Surface classification, focus/hotkey routing and game-only capture |

Hudhook renders Dear ImGui, so using it does not by itself solve HTML rendering. Qt/CXX-Qt remains the shared host and UI. Steam/Mangoapp coexistence, display resizing, non-default scaling, protected games and broad game coverage are not certified by the initial Faxanadu test.

## Reproduce the compositor test

```sh
nix develop --command scripts/test-gamescope
```

This launches a dedicated X11 fixture and the real Qt/WebEngine UI in a private Gamescope session. It checks game keyboard delivery, focus transfer/restoration, icon clicking/dragging, HTML widget drag/resize persistence, clean game-only capture, Escape, visibility shortcuts and session commands. Screenshots remain in the printed temporary directory. Run these diagnostics serially: Gamescope's legacy X11 screenshot request writes `/tmp/gamescope.png`.

The separate `capture_probe` example captures the launched game's pixmap and Gamescope's composed output on a supplied `DISPLAY`. Native runtime evidence and remaining gates are tracked in [validation](validation.md).
