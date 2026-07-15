{ config, pkgs ? import <nixpkgs> {}, lib, ... }:
let
  evalConfig = import (pkgs.path + "/nixos/lib/eval-config.nix");
  installer_base = pkgs.callPackage ./installer_base.nix {};
in
{
  options = {
    pxe-install-console = lib.mkOption {
      type = lib.types.str;
      default = "/dev/tty1";
      description = "Terminal to output text to";
    };
    pxe-install-modules = lib.mkOption {
      default = [];
      description = "Additional modules for the netboot installer";
    };
  };

  config =
  let
    super = config;
  in {
    system.build.installer_netboot = let
      build = (evalConfig {
        system = pkgs.system;
        modules = [
          (import "${pkgs.path}/nixos/modules/installer/netboot/netboot-minimal.nix")
          ({config, pkgs, lib, ...}: {
            netboot.squashfsCompression = "zstd -Xcompression-level 6";

            # Makes the console print to the main display.
            services.journald.console = super.pxe-install-console;

            # We need to set a system state version, so we'll just inherit it.
            system.stateVersion = super.system.stateVersion;
          })
        ] ++ super.pxe-install-modules;
      }).config.system.build;
    in pkgs.runCommand "netboot" {} ''
        mkdir -p $out
        ln -s ${build.kernel} $out/kernel
        ln -s ${build.netbootRamdisk} $out/netbootRamdisk
        ln -s ${build.toplevel} $out/toplevel
    '';
  };
}
