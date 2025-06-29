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

        # Use Python with tests disabled globally
        python = pkgs.python311.override {
          packageOverrides = final: prev: {
            buildPythonPackage =
              args:
              prev.buildPythonPackage (
                args
                // {
                  doCheck = false;
                }
              );
            buildPythonApplication =
              args:
              prev.buildPythonApplication (
                args
                // {
                  doCheck = false;
                }
              );
          };
        };

      in
      {
        packages.default = python.pkgs.buildPythonApplication {
          pname = "lazy-github";
          version = builtins.replaceStrings ["\n" "\"" " " "VERSION" "="] ["" "" "" "" ""] (builtins.readFile ./lazy_github/version.py);
          format = "pyproject";

          src = ./.;

          nativeBuildInputs = with python.pkgs; [
            hatchling
          ];

          propagatedBuildInputs = with python.pkgs; [
            httpx
            hishel
            pydantic
            textual
            click
          ];

          doCheck = false;

          meta = with pkgs.lib; {
            description = "A terminal UI for interacting with Github";
            homepage = "https://github.com/gizmo385/gh-lazy";
            license = licenses.mit;
            maintainers = [ ];
          };
        };

        # Development shell
        devShells.default = pkgs.mkShell {
          buildInputs = [
            python
            python.pkgs.pip
            python.pkgs.uv
          ];
        };
      }
    );
}

