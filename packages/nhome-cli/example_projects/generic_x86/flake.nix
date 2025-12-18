{
  description = "Generic x86 device example";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";

    # Tools for configuring hard drives and filesystems.
    disko = {
        url = "github:nix-community/disko/latest";
        inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, disko, ... }:
  let
    # Modules that are common between our system and our installers.
    networking = ({ pkgs, lib, config, ...}: {
      systemd.network.enable = true;
      networking.useNetworkd = true;
      networking.hostName = "generic-x86";
    });
    sshd = { pkgs, lib, config, ...}: {
      systemd.services.sshd.wantedBy = pkgs.lib.mkForce [ "multi-user.target" ];
      services.openssh.enable = true;
      services.openssh.settings.PermitRootLogin = "yes";
      users.extraUsers.root.openssh.authorizedKeys.keys = lib.splitString "\n" (builtins.readFile ./public-keys);
    };
  in
  {
    nixosConfigurations.generic-x86 = nixpkgs.lib.nixosSystem {  
      system = "x86_64-linux";
      modules = [
        # ../../nix_modules/basic_boot.nix
        disko.nixosModules.disko
	../../nix_modules/installer_iso.nix
	../../nix_modules/auto_revert.nix
        networking
        sshd
        ({ pkgs, lib, config, ...}: {
          system.stateVersion = "25.05";

          # Configures the network interface and ssh for the
          # installer PXE image. Try to keep these light and minimal,
          # since everything in the PXE image must be stored in the RAM
          # of the target device.
          pxe-install-modules = [
            networking
            sshd
          ];

          # Install system packages.
          environment.systemPackages = [
            pkgs.neovim
          ];
    
          # Install bootloader.
          boot.loader = {
            systemd-boot = {
                enable = true;
                configurationLimit = 4;
            };
            efi.canTouchEfiVariables = true;
          };

          # Configure the hard drive.
          disko.devices = {
            disk = {
              # Our main hard drive. You can give it a different name.
              main = {
                type = "disk";
                device = "/dev/sda";
                content = {
                  type = "gpt";
                  partitions = {
                    # Partition needed for UEFI booting.
                    ESP = {
                      priority = 1;
                      name = "ESP";
                      start = "1M";
                      end = "128M";
                      type = "EF00";
                      content = {
                        type = "filesystem";
                        format = "vfat";
                        mountpoint = "/boot";
                        mountOptions = [ "umask=0077" ];
                      };
                    };
                    # Our root filesystem.
                    root = {
                      size = "100%";
                      content = {
                        type = "btrfs";
                        extraArgs = [ "-f" ]; # Override existing partition
                        mountpoint = "/";
                        mountOptions = [
                          "compress=zstd"
                          "noatime"
                        ];
                      };
                    };
                  };
                };
              };
            };
          };

          services.beesd.filesystems = {
            store = {
              spec = "/";
              hashTableSizeMB = 2048;
              verbosity = "crit";
              extraOptions = [ "--loadavg-target" "5.0" ];
            };
          };
	})
	../../nix_modules/installer_netboot.nix
      ];
    };
  };
}
