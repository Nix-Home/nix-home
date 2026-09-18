# LXC template

A minimal, container-oriented NixOS configuration for building a Proxmox VE LXC template with nhome-cli. There is deliberately no bootloader, disko, or `fileSystems`: the `proxmox-lxc` image module sets `boot.isContainer`, enables sshd, and lets Proxmox manage the network.

## Usage

From the root of this project (or point `--project-root` at it):

```
nhome deploy lxc-template --project-root .
```

This builds one `tar.xz` LXC template per matching host (use `--hosts` to filter) and logs the path of each tarball. Import it into Proxmox with `pveam update <tarball>` or via the GUI.
