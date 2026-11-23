#!/bin/sh
# The workspace's references by the compiler's resolution (xrefs.py): rust-analyzer's SCIP index (macOS, the
# `test-hooks` feature set, as the tests build), the module tree of each crate's targets and the syn tool's `sites`.
# Run from the repository on a host with memory to spare (the index of the whole workspace), e.g.
#   scripts/dev/test-remote --os macos --memory 64 --fetch xrefs/xrefs.tgz -- sh ../tools/cratify/xrefs.sh
# Writes <target dir>/xrefs/xrefs.tgz: files.tsv, edges.tsv, cross.tsv (xrefs.py).
set -e
cd "$(dirname "$0")/../.."
T=${CARGO_TARGET_DIR:-$(pwd)/target}
O="$T/xrefs"
if [ "$1" = rejoin ]; then
  # The index made before, joined again (xrefs.py changed).
  python3 tools/cratify/xrefs.py join "$O"
  python3 tools/cratify/xrefs.py ties "$O"
  grep -m 20 'impl#' "$O/defs.tsv" | cut -f1,2 > "$O/impl-sample.tsv"
  tar -czf "$O/xrefs.tgz" -C "$O" files.tsv edges.tsv cross.tsv packages.tsv targets.tsv ties.tsv impl-sample.tsv
  du -h "$O/xrefs.tgz"
  exit 0
fi
rm -rf "$O"; mkdir -p "$O"
rustup component add rust-analyzer >/dev/null 2>&1 || true
RA=$(rustup which rust-analyzer)
cargo build -q --release --manifest-path tools/cratify/rs/Cargo.toml --target-dir "$T/cratify"
CRATIFY="$T/cratify/release/cratify"
cargo metadata --format-version 1 --no-deps > "$O/meta.json"
python3 - "$O" <<'PY'
import json, os, sys
o = sys.argv[1]
meta = json.load(open(f'{o}/meta.json'))
root = os.getcwd()
with open(f'{o}/packages.tsv', 'w') as p, open(f'{o}/targets.tsv', 'w') as t:
    for pkg in meta['packages']:
        d = os.path.relpath(os.path.dirname(pkg['manifest_path']), root)
        p.write(f"{pkg['name']}\t{d}\n")
        for tg in pkg['targets']:
            kind = tg['kind'][0]
            if kind in ('lib', 'rlib', 'proc-macro', 'bin'):
                t.write(f"{pkg['name']}\t{kind}\t{os.path.relpath(tg['src_path'], root)}\n")
PY
: > "$O/trees.tsv"
while IFS="$(printf '\t')" read -r name kind src; do
  "$CRATIFY" tree "$src" 2>/dev/null | sed "s/^/$name	$kind	/" >> "$O/trees.tsv" || echo "tree: $src failed" >&2
done < "$O/targets.tsv"
cut -f2 "$O/packages.tsv" | while read -r d; do find "$d" -name '*.rs' -not -path '*/target/*'; done | sort -u > "$O/rs.txt"
xargs "$CRATIFY" sites < "$O/rs.txt" > "$O/sites.tsv" 2> "$O/sites.err" || true
echo '{"cargo": {"features": ["test-hooks"]}}' > "$O/ra.json"
start=$(date +%s)
"$RA" scip . --output "$O/index.scip" --config-path "$O/ra.json" > "$O/ra.log" 2>&1 || { tail -20 "$O/ra.log"; exit 1; }
echo "index: $(( $(date +%s) - start )) s, $(du -h "$O/index.scip" | cut -f1)"
python3 tools/cratify/scip_decode.py "$O/index.scip" "$O"
python3 tools/cratify/xrefs.py join "$O"
python3 tools/cratify/xrefs.py ties "$O"
tar -czf "$O/xrefs.tgz" -C "$O" files.tsv edges.tsv cross.tsv packages.tsv targets.tsv ties.tsv
du -h "$O/xrefs.tgz"
