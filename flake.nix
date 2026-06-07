{
  description = "wallrack — picker-agnostic wallpaper manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        wallrack = pkgs.rustPlatform.buildRustPackage {
          pname = "wallrack";
          version = "0.1.0";

          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let base = baseNameOf path; in
              !(builtins.elem base [ "target" ".devenv" ".direnv" "result" ]);
          };

          cargoLock.lockFile = ./Cargo.lock;

          # All Rust deps are pure-Rust (image, jpeg-decoder, sha2, nix, notify,
          # …); no system libs to link. Runtime tools (awww/swww,
          # linux-wallpaperengine, hyprctl, matugen, etc.) are expected on the
          # user's PATH — wallrack invokes them as commands.

          # Ship the rofi reference frontend next to the binary so users who
          # want it can wire `wallrack-rofi-picker` straight into their hotkey.
          postInstall = ''
            install -Dm755 picker/wallrack_rofi_picker.sh $out/bin/wallrack-rofi-picker
          '';

          meta = with pkgs.lib; {
            description = "Picker-agnostic wallpaper manager (rofi/JSON output)";
            mainProgram = "wallrack";
            platforms = platforms.linux;
          };
        };
      in
      {
        packages.wallrack = wallrack;
        packages.default = wallrack;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ wallrack ];
          packages = with pkgs; [
            rust-analyzer
            rustfmt
            clippy
          ];
        };

        checks.build = wallrack;
      })
    // {
      overlays.default = final: prev: {
        wallrack = self.packages.${prev.system}.wallrack;
      };
    };
}
