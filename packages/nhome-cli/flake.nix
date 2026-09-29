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
          completionOverlay = ./completions/hosts-overlay.bash;
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

              # Generate the bash completion script from the command line
              # definition at build time, rename the generated function so it
              # can be wrapped by the dynamic host name overlay, and ship the
              # result where NixOS bash-completion will auto-load it.
              mkdir -p $out/share/bash-completion/completions $TMPDIR
              ${package}/bin/cli __completions bash 2> /dev/null \
                | sed 's/_nhome/_nhome_static/g' > $TMPDIR/nhome.bash
              cat ${completionOverlay} >> $TMPDIR/nhome.bash
              install -m 0644 $TMPDIR/nhome.bash $out/share/bash-completion/completions/nhome
	    '';
      }
    );
}
