{
  description = "Nix-Home command line tool";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
	craneLib = crane.mkLib pkgs;
      in
      {
        devShells.default = with pkgs; mkShell {
          buildInputs = [
	    bashInteractive
            openssl
            pkg-config
            pixiecore
            nixos-rebuild
            nixos-anywhere

            (rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
                "rustfmt"
                "clippy"
              ];
	    })
          ];

	  shellHook = ''
            export SHELL=${pkgs.bashInteractive}/bin/bash
          '';
        };

        packages.default =  with pkgs;
	  let
	    package = craneLib.buildPackage {
              src = craneLib.cleanCargoSource ./.;
          
	      strictDeps = true;
            };
	  in
	    pkgs.runCommandLocal "nhome-cli" {
	      nativeBuildInputs = [
                pkgs.makeWrapper
              ];
	    } ''
              mkdir -p $out/bin
              cp ${package}/bin/cli $out/bin/nhome
              wrapProgram $out/bin/nhome \
                --prefix PATH : ${pkgs.nix}/bin:${pkgs.nixos-rebuild}/bin:${pkgs.openssh}/bin:${pkgs.pixiecore}/bin:${pkgs.nixos-anywhere}/bin:{}
	    '';
      }
    );
}
