#!/usr/bin/env bash
set -euo pipefail

binary=$(realpath -- "${1:?需要 fundeploy 二进制路径}")
output=$(realpath -m -- "${2:?需要截图路径}")
work=$(mktemp -d)
socket="$work/tmux.sock"
trap 'tmux -S "$socket" kill-server 2>/dev/null || true; rm -rf -- "$work"' EXIT

tmux -S "$socket" -f /dev/null new-session -d -x 66 -y 18 -s menu \
    env -u CI -u NONINTERACTIVE -u NO_COLOR TERM=xterm-256color "$binary"
tmux -S "$socket" set-option -t menu status off
for ((attempt = 0; attempt < 50; attempt++)); do
    menu=$(tmux -S "$socket" capture-pane -p -t menu)
    case "$menu" in
        *fundeploy*service*monitor*) break ;;
    esac
    sleep 0.1
done
case "$menu" in
    *fundeploy*service*monitor*) ;;
    *) printf '%s\n' "未看到 fundeploy 主菜单" >&2; exit 1 ;;
esac

xvfb-run -a -e /dev/stderr -s '-screen 0 1200x720x24' bash -euc '
    xterm -T fundeploy -geometry 66x18 -b 16 -bg "#111318" -fg "#e4e8f0" \
        -fa "DejaVu Sans Mono" -fs 16 -xrm "XTerm*faceNameDoublesize: Noto Sans Mono CJK SC" \
        -e tmux -S "$1" attach-session -t menu &
    terminal=$!
    trap "kill $terminal 2>/dev/null || true" EXIT
    window=$(timeout 10s xdotool search --sync --onlyvisible --name "^fundeploy$")
    sleep 0.5
    scrot --window "$window" "$2"
' capture "$socket" "$work/menu.png"

mkdir -p -- "$(dirname -- "$output")"
mv -- "$work/menu.png" "$output"
