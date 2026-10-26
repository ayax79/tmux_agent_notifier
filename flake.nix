{
  description = "tmux-agent-notifier: surface Claude Code, Cursor, and Goose activity in tmux";

  inputs.nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          inherit (cargoToml.package) version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            inherit (cargoToml.package) description;
            mainProgram = "tan";
          };
        };
      });

      homeManagerModules.default = import ./nix/hm-module.nix self;

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
          ];
        };
      });

      checks = forAllSystems (
        pkgs:
        let
          src = pkgs.lib.cleanSource ./.;
          package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          runCheck =
            name: packages: script:
            pkgs.runCommand "tan-${name}" { nativeBuildInputs = packages; } ''
              cd ${src}
              ${script}
              touch $out
            '';
        in
        {
          inherit package;

          rustfmt = runCheck "rustfmt" [
            pkgs.cargo
            pkgs.rustfmt
          ] "cargo fmt --check";

          nixfmt = runCheck "nixfmt" [
            pkgs.nixfmt-rfc-style
            pkgs.findutils
          ] "find . -name '*.nix' -exec nixfmt --check {} +";

          typos = runCheck "typos" [ pkgs.typos ] "typos";

          clippy = package.overrideAttrs (old: {
            pname = "${old.pname}-clippy";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
            buildPhase = ''
              runHook preBuild
              cargo clippy --offline --all-targets -- -D warnings
              runHook postBuild
            '';
            doCheck = false;
            installPhase = "touch $out";
            dontFixup = true;
          });
        }
      );

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}
