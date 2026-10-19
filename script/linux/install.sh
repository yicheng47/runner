#!/bin/sh
set -eu
prefix=${HOME:?}/.local
uninstall=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix) [ "$#" -ge 2 ] || exit 2; prefix=$2; shift 2 ;;
        --uninstall) uninstall=true; shift ;;
        *) echo "Usage: install.sh [--prefix ABSOLUTE_PATH] [--uninstall]" >&2; exit 2 ;;
    esac
done
while [ "$prefix" != / ] && [ "${prefix%/}" != "$prefix" ]; do prefix=${prefix%/}; done
case "$prefix/" in *'/../'*|*'/./'*|*'//'*) echo 'Prefix must use a normalized absolute path' >&2; exit 2 ;; esac
case "$prefix" in /*) ;; *) echo 'Prefix must be absolute' >&2; exit 2 ;; esac
case "$prefix" in /|/usr|/usr/local) echo 'Use the system package manager for system prefixes' >&2; exit 2 ;; esac
case "$prefix" in *=*) echo 'Prefix must not contain an equals sign (desktop executable path)' >&2; exit 2 ;; esac
case "$prefix" in *'
'*) echo 'Prefix must not contain newlines' >&2; exit 2 ;; esac
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
package=$prefix/lib/runner
unit=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/runnerd.service
managed() { [ ! -L "$package" ] && [ -f "$package/install-layout-v1" ] && [ "$(cat "$package/install-layout-v1")" = runner-linux-package-v1 ]; }
owned_link() { [ -L "$1" ] && [ "$(readlink "$1")" = "$2" ]; }
if "$uninstall"; then
    managed || { echo 'No managed Runner installation at this prefix' >&2; exit 1; }
    for name in runner runnerd runner-app; do
        case "$name" in runner) target=runner-agent-cli ;; runnerd) target=runnerd ;; runner-app) target=Runner ;; esac
        owned_link "$prefix/bin/$name" "../lib/runner/$target" && rm "$prefix/bin/$name" || :
    done
    if [ ! -L "$unit" ] && [ -f "$unit" ] && grep -Fqx '# Runner managed user service' "$unit" && grep -Fqx "# Runner prefix: $prefix" "$unit"; then
        rm "$unit"
    fi
    desktop=$prefix/share/applications/runner.desktop
    if [ ! -L "$desktop" ] && [ -f "$desktop" ] && grep -Fqx "X-Runner-Prefix=$prefix" "$desktop"; then rm "$desktop"; fi
    icon=$prefix/share/icons/hicolor/256x256/apps/runner.png
    [ ! -L "$icon" ] && cmp -s "$icon" "$package/icon.png" && rm "$icon" || :
    rm -rf -- "$package"
    echo 'Runner removed. User data and running sessions are retained. If enabled, stop/disable runnerd before uninstalling; run systemctl --user daemon-reload afterwards.'
    exit 0
fi
if [ -e "$package" ] || [ -L "$package" ]; then
    [ ! -L "$package" ] && managed || { echo 'Refusing to replace an unmanaged Runner directory' >&2; exit 1; }
fi
# Check all ownership before changing files.
for name in runner runnerd runner-app; do
    case "$name" in runner) target=runner-agent-cli ;; runnerd) target=runnerd ;; runner-app) target=Runner ;; esac
    link=$prefix/bin/$name
    if [ -e "$link" ] || [ -L "$link" ]; then
        owned_link "$link" "../lib/runner/$target" || { echo "Refusing to replace foreign command: $link" >&2; exit 1; }
    fi
done
if [ -e "$unit" ] || [ -L "$unit" ]; then
    if [ -L "$unit" ] || ! grep -Fqx '# Runner managed user service' "$unit" || ! grep -Fqx "# Runner prefix: $prefix" "$unit"; then
        echo "Refusing to replace foreign service: $unit" >&2; exit 1
    fi
fi
desktop=$prefix/share/applications/runner.desktop
if [ -e "$desktop" ] || [ -L "$desktop" ]; then
    if [ -L "$desktop" ] || ! grep -Fqx "X-Runner-Prefix=$prefix" "$desktop"; then
        echo "Refusing to replace foreign desktop entry: $desktop" >&2; exit 1
    fi
fi
icon=$prefix/share/icons/hicolor/256x256/apps/runner.png
if [ -e "$icon" ] || [ -L "$icon" ]; then
    if [ -L "$icon" ] || ! managed || ! cmp -s "$icon" "$package/icon.png"; then
        echo "Refusing to replace foreign icon: $icon" >&2; exit 1
    fi
fi
mkdir -p "$prefix/lib" "$prefix/bin" "$prefix/share/applications" "$prefix/share/icons/hicolor/256x256/apps" "$(dirname "$unit")"
stage=$(mktemp -d "$prefix/lib/.runner-install.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
cp -R "$root/lib/runner/." "$stage/"
# Each executable is replaced by rename, so a running daemon keeps its old inode.
mkdir -p "$package"
for file in "$stage"/*; do mv -f -- "$file" "$package/"; done
for name in runner runnerd runner-app; do
    case "$name" in runner) target=runner-agent-cli ;; runnerd) target=runnerd ;; runner-app) target=Runner ;; esac
    [ -L "$prefix/bin/$name" ] || ln -s "../lib/runner/$target" "$prefix/bin/$name"
done
escaped=$(printf '%s' "$prefix" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g; s/[$]/$$/g')
{
    printf '# Runner prefix: %s\n' "$prefix"
    while IFS= read -r line; do
        case "$line" in ExecStart=*) printf 'ExecStart="%s/lib/runner/runnerd"\n' "$escaped" ;; *) printf '%s\n' "$line" ;; esac
    done < "$root/lib/systemd/user/runnerd.service"
} > "$unit"
desktop_escaped=$(printf '%s' "$prefix" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g' | sed 's/[$`]/\\&/g')
desktop_escaped=$(printf '%s' "$desktop_escaped" | sed 's/\\/\\\\/g')
{
    while IFS= read -r line; do
        case "$line" in Exec=*) printf 'Exec="%s/bin/runner-app"\n' "$desktop_escaped" ;; *) printf '%s\n' "$line" ;; esac
    done < "$root/share/applications/runner.desktop"
    printf 'X-Runner-Prefix=%s\n' "$prefix"
} > "$prefix/share/applications/runner.desktop"
cp "$root/share/icons/hicolor/256x256/apps/runner.png" "$prefix/share/icons/hicolor/256x256/apps/runner.png"
echo "Installed Runner under $prefix. Add $prefix/bin to PATH."
echo 'The user service is not enabled. Run systemctl --user daemon-reload before optionally starting runnerd.'
