#!/bin/sh
# Probe script executed on a NixOS host by `nhome status`.
# Emits one "KEY value" line per datum. The format is parsed by
# parse_probe_output in src/status/mod.rs; keep the two in sync.

set -u

# Make sure the NixOS system binaries are found no matter how sshd is
# configured to set up its environment.
PATH="/run/current-system/sw/bin:${PATH}"

# 1. Overall systemd state. `is-system-running` exits non-zero for anything
#    but "running", so use its output instead of its exit status.
state=$(systemctl is-system-running 2>/dev/null)
[ -n "$state" ] || state=unknown
printf 'STATE %s\n' "$state"

# 2. The system that is currently running.
current=$(readlink -f /run/current-system 2>/dev/null)
[ -n "$current" ] || current=unknown
printf 'CURRENT %s\n' "$current"

# 3. The system the bootloader will boot next (systemd-boot). NixOS writes
#    "default <entry>" to loader.conf on the ESP and points the entry's
#    options line at the store path of that system via init=.
boot=""
for loader_conf in /boot/loader/loader.conf /efi/loader/loader.conf; do
    [ -r "$loader_conf" ] || continue
    default=$(awk '/^default[ \t]+/ { print $2; exit }' "$loader_conf" 2>/dev/null)
    [ -n "$default" ] || continue
    entry="$(dirname "$loader_conf")/entries/$default"
    [ -r "$entry" ] || continue
    init=$(awk '/^options[ \t]+/ { for (i = 1; i <= NF; i++) if ($i ~ /^init=/) { print substr($i, 6); exit } }' "$entry" 2>/dev/null)
    [ -n "$init" ] || continue
    boot=${init%/init}
    break
done

if [ -n "$boot" ]; then
    boot=$(readlink -f "$boot" 2>/dev/null)
fi
[ -n "$boot" ] || boot=none
printf 'BOOT %s\n' "$boot"

# 4. Whether we are in a container (containers have no bootloader).
#    `systemd-detect-virt -c` prints "none" (exit 0) when not in one.
container=$(systemd-detect-virt -c 2>/dev/null)
[ "$container" = "none" ] && container=""
printf 'CONTAINER %s\n' "$container"

# 5. Best-effort generation numbers: match the store paths against the
#    system profile links in /nix/var/nix/profiles/system-<n>-link.
gen_for() {
    p=$1
    case $p in
        /nix/store/*) ;;
        *) return 1 ;;
    esac
    for link in /nix/var/nix/profiles/system-[0-9]*-link; do
        if [ "$(readlink -f "$link" 2>/dev/null)" = "$p" ]; then
            n=${link##*/system-}
            printf '%s' "${n%-link}"
            return 0
        fi
    done
    return 1
}

if gen=$(gen_for "$current"); then
    printf 'CURRENT_GEN %s\n' "$gen"
fi
if [ "$boot" != "none" ] && gen=$(gen_for "$boot"); then
    printf 'BOOT_GEN %s\n' "$gen"
fi
