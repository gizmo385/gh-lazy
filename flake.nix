{
  description = "A terminal UI for interacting with Github";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        python = pkgs.python311;
        
        textual-fspicker = python.pkgs.buildPythonPackage rec {
          pname = "textual-fspicker";
          version = "0.4.1";
          format = "pyproject";

          src = pkgs.fetchPypi {
            inherit pname version;
            sha256 = "sha256-1iaak97il3aydz3li5ggy3njj2pmxf8hvcfvjpl4qjw6q54i26l6=";
          };

          nativeBuildInputs = with python.pkgs; [
            hatchling
          ];

          propagatedBuildInputs = with python.pkgs; [
            textual
          ];
        };
      in
      {
        packages.default = python.pkgs.buildPythonApplication {
          pname = "lazy-github";
          version = "0.1.0";
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
            textual-fspicker
          ];

          meta = with pkgs.lib; {
            description = "A terminal UI for interacting with Github";
            homepage = "https://github.com/gizmo385/gh-lazy";
            license = licenses.mit;
            maintainers = [ ];
          };
        };

        # Development shell
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            python
            python.pkgs.pip
            python.pkgs.uv
          ];
        };
      });
}