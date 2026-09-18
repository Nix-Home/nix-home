{
  description = "LXC template example";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };
  outputs = { self, nixpkgs, ... }:
  {
    nixosConfigurations.lxc = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        # A minimal, container-oriented configuration. Deliberately no
        # bootloader, disko, or fileSystems: the proxmox-lxc image module
        # sets `boot.isContainer`, enables sshd, and lets Proxmox manage
        # the network.
        ({ pkgs, lib, config, ... }: {
          networking.hostName = "lxc";

          services.openssh.enable = true;
          services.openssh.settings.PermitRootLogin = "yes";
          users.extraUsers.root.openssh.authorizedKeys.keys =
            lib.splitString "\n" (builtins.readFile ./public-keys);

          environment.systemPackages = [
            pkgs.neovim
            pkgs.git
          ];

          system.stateVersion = "25.05";
        })
      ];
    };
  };
}
