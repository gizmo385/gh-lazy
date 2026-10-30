{
  description = "A terminal UI for interacting with Github";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        inherit (pkgs) lib;
        cargoToml = lib.importTOML ./Cargo.toml;
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "lazy-github";
          version = cargoToml.workspace.package.version;

          # Only what cargo reads, so edits to the Python app, parity/, docs/ or target/ don't rebuild the package.
          # xtask stays in because it's a workspace member and cargo reads every member's manifest.
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
              ./xtask
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;

          # cargoInstallHook installs every executable in the release directory, so build only the app: bin/ must
          # hold just lazy-github, not fake-gh, fake-ssh or xtask.
          cargoBuildFlags = [
            "-p"
            "lazy-github"
          ];

          # Tests read parity/ fixtures (dropped by the source filter), some drive a PTY, and all of them already
          # run in CI (rust.yaml).
          doCheck = false;

          nativeBuildInputs = [ pkgs.makeWrapper ];

          # --suffix, not --prefix: Nix's gh and git are a fallback, the user's own (with their extensions) win.
          postInstall = ''
            wrapProgram $out/bin/lazy-github \
              --suffix PATH : ${
                lib.makeBinPath [
                  pkgs.gh
                  pkgs.git
                ]
              }
          '';

          meta = {
            description = "A terminal UI for interacting with Github";
            homepage = "https://github.com/gizmo385/gh-lazy";
            license = lib.licenses.mit;
            mainProgram = "lazy-github";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = [
            # rustup honors rust-toolchain.toml, the same toolchain CI installs.
            pkgs.rustup
            pkgs.gh
            pkgs.git
            # uv and Python run the parity harness's Python reference until cutover; #207 removes them.
            pkgs.uv
            pkgs.python313
          ];

          # uv's downloaded Pythons don't run on NixOS; 3.13 is what CI uses for the reference.
          UV_PYTHON_DOWNLOADS = "never";
          UV_PYTHON = "${pkgs.python313}/bin/python3";
        };
      }
    );
}
