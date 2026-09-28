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
        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [ cargo rustc rustfmt clippy pkg-config clang cmake ninja ffmpeg yt-dlp whisper-cpp watchexec ])
            ++ qtModules ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.alsa-lib pkgs.libxcb-cursor pkgs.mold pkgs.pipewire pkgs.pulseaudio pkgs.gamescope ];
          QMAKE = "${qtEnv}/bin/qmake";
          FLAKE_INPUTS = builtins.concatStringsSep ":" (map (input: input.outPath) (builtins.attrValues (builtins.removeAttrs inputs [ "self" ])));
          shellHook = ''
            export QMAKE="${qtEnv}/bin/qmake"
            export PATH="${qtEnv}/bin:${qtEnv}/libexec:$PATH"
            export QT_INCLUDE_PATH="${qtEnv}/include"
            export QT_LIBEXEC_PATH="${qtEnv}/libexec"
            export QT_PLUGIN_PATH="${qtEnv}/lib/qt-6/plugins"
            export QML_IMPORT_PATH="${qtEnv}/lib/qt-6/qml"
            export QT_QUICK_CONTROLS_STYLE=Basic
            export QT_FORCE_STDERR_LOGGING=1
            ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.pipewire pkgs.alsa-lib]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            ''}
          '';
        };
      });
}
