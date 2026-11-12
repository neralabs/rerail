#!/bin/sh
# Demo: open capability_store.py in Neovim (in tmux) with `rerail lsp`, apply the unsaved edit, and write the screen
# as ANSI and HTML once its diagnostics arrive.
# Usage: capture.sh <engine checkout> <data-layer checkout> <rerail home> <float 0|1> <output prefix>
set -eu
engine=$1 checkout=$2 home=$3 float=$4 out=$5
here=$(cd "$(dirname "$0")" && pwd)
session=rerail-lsp-demo-$$
tmux new-session -d -s "$session" -x 172 -y 34 -c "$checkout" \
  -e DEMO_ENGINE="$engine" -e DEMO_HOME="$home" -e DEMO_FLOAT="$float" \
  "nvim -u '$here/init.lua' l6500_evidence/capability_store.py; sleep 60"
trap 'tmux kill-session -t "$session" 2>/dev/null || true' EXIT
sleep 3
tmux send-keys -t "$session" ":lua dofile('$here/edit.lua')" Enter
for _ in $(seq 1 100); do
  sleep 0.2
  tmux capture-pane -p -t "$session" | grep -q 'lsp: rerail' || continue
  # The float, or the virtual text, is on screen once the diagnostics arrived.
  tmux capture-pane -p -t "$session" | grep -q 'sync-delegation' && break
done
sleep 0.5
tmux capture-pane -p -e -t "$session" > "$out.ansi"
python3 "$here/ansi2html.py" "$out.ansi" '#e0e2ea' '#14161b' \
  "nvim — l6500_evidence/capability_store.py (data-layer @ 2b1504ad) — rerail lsp" > "$out.html"
tmux capture-pane -p -t "$session"
