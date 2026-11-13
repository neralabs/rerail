#!/bin/sh
# The crate split's settling pass, after apply.py (regenerate.sh runs it): checks each generated crate in the
# DAG's order, then the engine's test build (every crate under test-hooks), then every crate's own tests (each builds
# alone under `cargo test --workspace`: a test reads its own crate and those below), and on errors applies the sweeps
# the compiler's reports call for, up to 8 tries each:
#   prune_facades.py (a facade's repeated use of what another crate keeps private or lacks),
#   privtypes.py (types, fields and parent items reported private), defsite.py (at the definitions rustc names).
# Stops at the first crate that does not settle, its errors in <work dir>/ck-<crate>.txt.
#
#   tools/cratify/settle.sh <work dir>        (in the tree's root; CRATIFY_SOURCE: a second tree kept in step)
W=$1
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(cd "$HERE/../.." && pwd)
cd "$ROOT"
ORDER=$(cd "$HERE" && python3 -c "
from crate_map import DAG
done, out = set(), []
def visit(c):
    if c in done: return
    done.add(c)
    for d in DAG.get(c, ()): visit(d)
    out.append(c)
for c in DAG: visit(c)
print(' '.join(c for c in out if c != 'rerail-engine') + ' rerail-engine')")
count() { grep -cE ': error(\[|:)|^error' "$1" || true; }
sweep() {  # check output (short), the human-format command
  python3 "$HERE/prune_facades.py" "$1" > /dev/null
  python3 "$HERE/privtypes.py" "$1" "$ROOT" "$W/tree2.tsv" > /dev/null
  eval "$2" > "$W/ck-human.txt" 2>&1; python3 "$HERE/defsite.py" "$W/ck-human.txt" "$ROOT" > /dev/null
}
for c in $ORDER test-build crate-tests; do
  case $c in
    test-build) args="-p rerail-engine --lib --tests" ;;
    crate-tests) args="--workspace --tests --keep-going" ;;
    *) args="-p $c --lib" ;;
  esac
  for k in 1 2 3 4 5 6 7 8; do
    nice -n 19 cargo check $args --message-format short > "$W/ck-$c.txt" 2>&1
    n=$(count "$W/ck-$c.txt")
    [ "$n" = 0 ] && break
    sweep "$W/ck-$c.txt" "nice -n 19 cargo check $args"
  done
  echo "$c errors=$n"
  [ "$n" = 0 ] || { grep -E ': error' "$W/ck-$c.txt" | head -20; exit 1; }
done
