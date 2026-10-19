#!/bin/sh
# Embedded in native package maintainer scripts; DPKG_ROOT supports isolated dpkg validation.
set -eu
action=$1
prefix=${DPKG_ROOT:-}/usr
owned() { [ -L "$1" ] && [ "$(readlink "$1")" = "$2" ]; }
if [ "$action" = check ] || [ "$action" = install ]; then
    for name in runner runnerd runner-app; do
        case "$name" in runner) target=runner-agent-cli ;; runnerd) target=runnerd ;; runner-app) target=Runner ;; esac
        link=$prefix/bin/$name
        if [ -e "$link" ] || [ -L "$link" ]; then
            owned "$link" "../lib/runner/$target" || { echo "Refusing to replace foreign command: $link" >&2; exit 1; }
        fi
    done
fi
if [ "$action" = install ]; then
    mkdir -p "$prefix/bin"
fi
for name in runner runnerd runner-app; do
    case "$name" in runner) target=runner-agent-cli ;; runnerd) target=runnerd ;; runner-app) target=Runner ;; esac
    link=$prefix/bin/$name
    case "$action" in
        check) : ;;
        install) [ -L "$link" ] || ln -s "../lib/runner/$target" "$link" ;;
        remove) if owned "$link" "../lib/runner/$target"; then rm "$link"; fi ;;
        *) echo "Invalid Runner package alias action: $action" >&2; exit 2 ;;
    esac
done
