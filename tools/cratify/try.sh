#!/bin/sh
# try.sh "prefix<TAB>crate" ... : the join with these lines added to the map (the map itself is untouched)
S=/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Cratification/2e6a54e4-c96c-4d03-beeb-ec37d97e21da/scratchpad
cp crate-map.tsv $S/map.bak; for l in "$@"; do printf '%b\n' "$l" >> crate-map.tsv; done
(cd $S && python3 $OLDPWD/join.py edges.json out-try idx-all idx-wasm) | sed -n 2p
mv $S/map.bak crate-map.tsv
