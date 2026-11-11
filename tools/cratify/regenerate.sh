#!/bin/sh
# The crate split's generators, in order, on the tree this script is in (a rehearsal worktree, or the prep
# branch at the stop). Idempotent inputs: the index of this same tree, unedited.
#
#   tools/cratify/regenerate.sh <index dir> <work dir>
#
# <index dir>: reindex output (idx-macos/ and idx-wasm/, decoded with scip_decode.py) of this tree as it is.
# <work dir>:  where the module graph, the joins and the logs go.
# CRATIFY_BIN: the syn tool (tools/cratify/rs, built with its own --target-dir).
#
# 1. the module graph and tree; 2. the join without test hosts, and the hosts it implies (test-hosts.tsv);
# 3. the join with them; 4. widen what other crates read (index positions: before any other edit);
# 5. the tree again; 6. the mounts, manifests and file edits.
set -e
IDX=$1; W=$2
: "${CRATIFY_BIN:?set CRATIFY_BIN to the syn tool}"
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(cd "$HERE/../.." && pwd)
mkdir -p "$W"
cd "$W" && python3 "$HERE/modtree.py" "$ROOT/native/src" > /dev/null && cd "$ROOT"
"$CRATIFY_BIN" tree native/src/lib.rs > "$W/tree.tsv"
rm -f "$HERE/test-hosts.tsv"
python3 "$HERE/join.py" "$W/edges.json" "$W/join0" "$IDX/idx-macos" "$IDX/idx-wasm" | head -2
python3 "$HERE/hosts.py" "$W/tree.tsv" "$W/join0/tests.tsv"
python3 "$HERE/join.py" "$W/edges.json" "$W/join" "$IDX/idx-macos" "$IDX/idx-wasm" | head -2
python3 "$HERE/widen.py" "$W/join/widen.tsv" --apply > "$W/widen.txt" 2>&1 || true
head -1 "$W/widen.txt"; echo "widen refused: $(grep -c REFUSED "$W/widen.txt" || true)"
"$CRATIFY_BIN" tree native/src/lib.rs > "$W/tree2.tsv"
python3 "$HERE/apply.py" "$W/tree2.tsv" "$W/join" --write | tail -2
