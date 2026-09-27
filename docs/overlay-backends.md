# Overlay backend decision

September 27, 2026. Shared Rust host and HTML UI, separate compositor/render integration per platform. Current code is a desktop-window interaction prototype; the native backends below are not implemented.

## Steam and Gamescope

Valve documents that the [Steam overlay hooks graphics device creation](https://partner.steamgames.com/doc/features/overlay), supports DirectX 7–12, OpenGL, Metal and Vulkan, and is injected into the game. Steamworks opens Steam's own UI; it is not an arbitrary GameBuddy widget renderer. Reproduce the integration approach using reusable components rather than treating Steam's proprietary overlay as an SDK.

[Gamescope](https://github.com/ValveSoftware/gamescope) is a Linux compositor that runs nested on a desktop or as a dedicated session. It can compose game and overlay surfaces without depending on the host desktop's always-on-top policy. It is not Steam's Windows/macOS overlay renderer.

| Platform | Backend to develop | Boundary |
| --- | --- | --- |
| Linux | Game and GameBuddy overlay in one Gamescope session | Composition, input routing and capture need integration |
| Windows | Thin graphics-hook DLL plus out-of-process Rust host | Evaluate [hudhook](https://github.com/veeenu/hudhook) for D3D9/11/12 and OpenGL3; Vulkan needs separate work |
| macOS | Metal/OpenGL presentation adapter with native capture/input | Signing, hardened-runtime compatibility and supported games need native verification |

Hudhook renders Dear ImGui. HTML needs an offscreen browser texture and input bridge. Retain Qt/CXX-Qt for the host/settings while proving an offscreen HTML producer for injected backends; [Qt WebEngine constraints](https://doc.qt.io/qt-6/qtwebengine-platform-notes.html) need consideration. Do not assume a QML scene can simply be transplanted into a game swapchain.

## Linux contract

Intended launch: Lunchpail → GameBuddy session coordinator → Gamescope → game and overlay clients. Preserve executable/argv/environment/current directory, report actual game process identity and retain Lunchpail's controller/save/session ownership. The current post-launch companion hook cannot retroactively put a game into a new compositor session.

Gamescope's [compositor source](https://github.com/ValveSoftware/gamescope/blob/master/src/steamcompmgr.cpp) contains `STEAM_OVERLAY`, `GAMESCOPE_EXTERNAL_OVERLAY` and `STEAM_INPUT_FOCUS`. These are implementation details, not a guaranteed independent interactive-overlay SDK. Displaying an external overlay does not establish correct input routing. [MangoHud recommends mangoapp for Gamescope](https://github.com/flightlessmango/MangoHud#gamescope); use it as a composition reference, not a complete conversational UI.

Prototype a version-pinned adapter and test coexistence with Steam/Mangoapp. A small maintained Gamescope extension may be necessary for interactive regions and game-only capture; decide from measured behavior. Do not globally reconfigure the user's compositor.

## Frames and input

The host produces alpha widget textures with surface IDs, dimensions, scale, dirty regions, session ID and sequence number. The backend uses the latest available texture without waiting on a model. Route pointer events only for the icon/activated widgets; closing the UI releases grabs and restores game focus. Speech should not steal game controls.

Capture game frames before GameBuddy composition, attach timestamps/epochs and drop obsolete frames under load. The synchronizer must not identify its own walkthrough video or text as gameplay. Inference/browser work stays outside the game process; model failure must not stall presentation.

The first proof is Faxanadu in an emulator: fullscreen composition, icon movement, widget resize, passthrough, focus restoration, game-only capture, compositor resize and clean exit. Repeat native acceptance per platform before advertising broad compatibility. Unsupported graphics APIs and protected-game restrictions must be explicit per backend.
