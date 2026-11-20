#!/bin/sh
# S=<scratch> try.sh "prefix<TAB>crate" ... : the join with these lines added to the map (the map itself is untouched).
# S is the folder that holds join.py's inputs (edges.json, idx-all, idx-wasm).
S="${S:?set S to the folder that holds edges.json, idx-all and idx-wasm}"
cp crate-map.tsv $S/map.bak; for l in "$@"; do printf '%b\n' "$l" >> crate-map.tsv; done
(cd $S && python3 $OLDPWD/join.py edges.json out-try idx-all idx-wasm) | sed -n 2p
mv $S/map.bak crate-map.tsv
