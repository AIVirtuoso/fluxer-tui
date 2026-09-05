{
  description = "fluxer-tui - a TUI chat client for the Fluxer messaging platform";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    cargoToml = nixpkgs.lib.importTOML ./Cargo.toml;
  in {
    packages = forAllSystems (pkgs: rec {
      fluxer-tui = pkgs.rustPlatform.buildRustPackage {
        pname = "fluxer-tui";
        version = cargoToml.package.version + (
          if self ? shortRev
          then "-${self.shortRev}"
          else "-dirty"
        );

        src = self;

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [pkgs.makeWrapper];

        # chafa is the text-art fallback when the terminal answers no
        # graphics-protocol query; sixel/kitty/iTerm2 rendering is built in.
        # wl-clipboard/xclip are looked up on PATH at runtime for Ctrl+V, so
        # whatever the desktop already has gets used.
        postInstall = ''
          wrapProgram $out/bin/fluxer-tui --suffix PATH : ${pkgs.lib.makeBinPath [pkgs.chafa]}
        '';

        meta = {
          description = "TUI chat client for the Fluxer messaging platform";
          homepage = "https://github.com/dogbonewish/fluxer-tui";
          license = pkgs.lib.licenses.mit;
          mainProgram = "fluxer-tui";
        };
      };
      default = fluxer-tui;
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [cargo rustc rustfmt clippy chafa wl-clipboard];
      };
    });
  };
}
