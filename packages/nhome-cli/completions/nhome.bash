# Tab completion for nhome.
#
# Installed at share/bash-completion/completions/nhome by the package, which
# bash-completion >= 2.x's lazy loader picks up automatically for any PATH
# entry that contains the nhome binary. No user configuration required.

_nhome_cache_root=""
_nhome_cache_ts=0
_nhome_cache_hosts=()

# Print the project hosts for a root directory, cached per-shell for 60s.
# Never prints to stderr and always succeeds (completion must stay quiet).
_nhome_hosts_for_root() {
    local root="$1"
    local now
    now=$(date +%s)

    if [ -n "${_nhome_cache_root}" ] && [ "${_nhome_cache_root}" = "${root}" ]; then
        if [ $((now - _nhome_cache_ts)) -lt 60 ]; then
            if [ ${#_nhome_cache_hosts[@]} -gt 0 ]; then
                printf '%s\n' "${_nhome_cache_hosts[@]}"
            fi
            return 0
        fi
    fi

    local hosts
    hosts=$(nhome __hosts --project-root "${root}" 2>/dev/null) || true

    _nhome_cache_root="${root}"
    _nhome_cache_ts="${now}"
    _nhome_cache_hosts=()
    if [ -n "${hosts}" ]; then
        readarray -t _nhome_cache_hosts <<< "${hosts}"
    fi

    if [ ${#_nhome_cache_hosts[@]} -gt 0 ]; then
        printf '%s\n' "${_nhome_cache_hosts[@]}"
    fi
    return 0
}

# Last --project-root value seen on the command line (defaults to CWD).
_nhome_root() {
    local i word
    local root="."
    for ((i = 1; i < ${#COMP_WORDS[@]}; i++)); do
        word="${COMP_WORDS[i]}"
        if [ "${word}" = "--project-root" ]; then
            if ((i + 1 < ${#COMP_WORDS[@]})); then
                root="${COMP_WORDS[i+1]}"
            fi
        elif [ "${word#--project-root=}" != "${word}" ]; then
            root="${word#--project-root=}"
        fi
    done
    printf '%s' "${root}"
}

_nhome() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local i word
    local sub="" sub2="" sub3=""
    local expecting_value="" has_ssh_host=0 has_op=0

    # Walk the words before the cursor to figure out where in the command
    # tree we are, skipping option values. (Index 0 is the command name.)
    for ((i = 1; i < COMP_CWORD; i++)); do
        word="${COMP_WORDS[i]}"

        if [ -n "${expecting_value}" ]; then
            expecting_value=""
            continue
        fi

        case "${word}" in
            -b|--build-machine|--hosts|--project-root|--link-path|--destination|-c|--command)
                expecting_value="yes"
                ;;
            --*)
                # Flags or attached-value options (e.g. --hosts=regex,
                # --host=10.0.0.1); they do not affect the command context.
                ;;
            *)
                if [ -z "${sub}" ]; then
                    sub="${word}"
                elif [ "${sub}" = "deploy" ]; then
                    if [ -z "${sub2}" ]; then
                        sub2="${word}"
                    elif [ "${sub2}" = "ssh" ]; then
                        has_op=1
                    elif [ "${sub2}" = "install-netboot" ] && [ -z "${sub3}" ]; then
                        sub3="${word}"
                    fi
                elif [ "${sub}" = "firewall" ] && [ -z "${sub2}" ]; then
                    sub2="${word}"
                elif [ "${sub}" = "ssh" ]; then
                    has_ssh_host=1
                fi
                ;;
        esac
    done

    # If the previous word is an option expecting a value, complete its value.
    if ((COMP_CWORD >= 1)); then
        local prev="${COMP_WORDS[COMP_CWORD-1]}"
        case "${prev}" in
            -b|--build-machine|-c|--command)
                COMPREPLY=()
                return 0
                ;;
            --hosts|--destination)
                COMPREPLY=( $(compgen -W "$(_nhome_hosts_for_root "$(_nhome_root)")" -- "${cur}") )
                return 0
                ;;
            --project-root)
                COMPREPLY=( $(compgen -d -- "${cur}") )
                return 0
                ;;
            --link-path)
                COMPREPLY=( $(compgen -d -- "${cur}") )
                return 0
                ;;
        esac
    fi

    case "${sub}" in
        "")
            if [ "${cur:0:1}" = "-" ]; then
                COMPREPLY=( $(compgen -W "-b --build-machine" -- "${cur}") )
            else
                COMPREPLY=( $(compgen -W "new deploy ssh firewall" -- "${cur}") )
            fi
            ;;
        new)
            COMPREPLY=()
            ;;
        deploy)
            case "${sub2}" in
                "")
                    if [ "${cur:0:1}" = "-" ]; then
                        COMPREPLY=( $(compgen -W "--hosts --project-root" -- "${cur}") )
                    else
                        COMPREPLY=( $(compgen -W "ssh disk install-iso install-netboot lxc-template --hosts --project-root" -- "${cur}") )
                    fi
                    ;;
                ssh)
                    if [ "${cur:0:1}" = "-" ]; then
                        COMPREPLY=( $(compgen -W "--no-auto-revert --destination" -- "${cur}") )
                    elif [ "${has_op}" = 0 ]; then
                        COMPREPLY=( $(compgen -W "test switch boot" -- "${cur}") )
                    else
                        COMPREPLY=()
                    fi
                    ;;
                disk|install-iso|lxc-template)
                    COMPREPLY=( $(compgen -W "--link-path" -- "${cur}") )
                    ;;
                install-netboot)
                    case "${sub3}" in
                        "")
                            COMPREPLY=( $(compgen -W "both boot install" -- "${cur}") )
                            ;;
                        install)
                            COMPREPLY=( $(compgen -W "--destination" -- "${cur}") )
                            ;;
                    esac
                    ;;
            esac
            ;;
        ssh)
            if [ "${cur:0:1}" = "-" ]; then
                COMPREPLY=( $(compgen -W "--project-root -c --command" -- "${cur}") )
            elif [ "${has_ssh_host}" = 0 ]; then
                COMPREPLY=( $(compgen -W "$(_nhome_hosts_for_root "$(_nhome_root)")" -- "${cur}") )
            else
                COMPREPLY=()
            fi
            ;;
        firewall)
            case "${sub2}" in
                "")
                    if [ "${cur:0:1}" = "-" ]; then
                        COMPREPLY=( $(compgen -W "--hosts --project-root" -- "${cur}") )
                    else
                        COMPREPLY=( $(compgen -W "disable reset pierce --hosts --project-root" -- "${cur}") )
                    fi
                    ;;
                pierce)
                    if [ "${cur:0:1}" = "-" ]; then
                        COMPREPLY=( $(compgen -W "--host" -- "${cur}") )
                    else
                        # IP addresses and host names are not enumerable.
                        COMPREPLY=()
                    fi
                    ;;
            esac
            ;;
    esac

    return 0
}

complete -F _nhome nhome
