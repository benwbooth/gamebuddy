{
  description = "GameBuddy Rust and Qt development environment";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  outputs = inputs@{ nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        qtModules = with pkgs.qt6; [ qtbase qtdeclarative qtmultimedia qtsvg qtspeech qttools qtwebengine ]
          ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ qtwayland ];
        qtEnv = pkgs.qt6.env "gamebuddy-qt" qtModules;
      in {
        packages = {
          prepared-speech-runtime = pkgs.python3.withPackages (ps: [ (ps.toPythonModule pkgs.piper-tts) ]);
          # Kokoro: a more natural local voice than Piper, on the CPU.
          kokoro-speech-runtime = pkgs.python3.withPackages (ps: [ ps.kokoro ps.soundfile ps.numpy ps.spacy-models.en_core_web_sm ]);
        };
        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [ cargo rustc rustfmt clippy pkg-config clang cmake ninja ffmpeg yt-dlp whisper-cpp watchexec tesseract ])
            ++ qtModules ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.alsa-lib pkgs.libxcb-cursor pkgs.mold pkgs.pipewire pkgs.pulseaudio pkgs.xwayland pkgs.wayland pkgs.libxkbcommon pkgs.libGL pkgs.libgbm pkgs.libdrm pkgs.libinput pkgs.udev pkgs.seatd ];
          QMAKE = "${qtEnv}/bin/qmake";
          FLAKE_INPUTS = builtins.concatStringsSep ":" (map (input: input.outPath) (builtins.attrValues (builtins.removeAttrs inputs [ "self" ])));
          shellHook = ''
            # A host-wide -L/usr/lib (nix-ld's system glibc) would link the
            # app against a newer glibc than the shell's Qt runtime provides.
            unset NIX_CFLAGS_LINK
            export QMAKE="${qtEnv}/bin/qmake"
            export PATH="${qtEnv}/bin:${qtEnv}/libexec:$PATH"
            export QT_INCLUDE_PATH="${qtEnv}/include"
            export QT_LIBEXEC_PATH="${qtEnv}/libexec"
            export QT_PLUGIN_PATH="${qtEnv}/lib/qt-6/plugins"
            export QML_IMPORT_PATH="${qtEnv}/lib/qt-6/qml"
            export QT_QUICK_CONTROLS_STYLE=Basic
            export QT_FORCE_STDERR_LOGGING=1
            ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.pipewire pkgs.alsa-lib pkgs.wayland pkgs.libxkbcommon pkgs.libGL pkgs.xorg.libX11 pkgs.xorg.libXcursor pkgs.xorg.libXi pkgs.xorg.libXrandr]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            ''}
          '';
        };
      });
}
