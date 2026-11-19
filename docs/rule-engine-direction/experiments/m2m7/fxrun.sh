#!/bin/zsh
# run m2exec over all fixture events and compare with native
cd /private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Pulling-it-all-together/3adb237a-22e7-4e0c-9a7a-33e109d2ab63/scratchpad/m2m7
B=$PWD/exec/target/release/m2exec
for i in $(seq 0 96); do $B fx/$i fx/$i.files fx/$i.mine.json ${1:-memo} 1 fx/$i.policy.json >fx/$i.log 2>&1 || echo "fail $i"; done
python3 fxcmp.py | tail -1
