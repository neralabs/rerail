#!/usr/bin/env bash
# Pass 1 superposed (decision 0698, item 100) has no branch: its block map and its composition in the block loop
# compile to no conditional branch and no call. Builds the comparison example (profiling) and reads its machine
# code. Run from native/. Needs otool (macOS) or objdump.
set -euo pipefail
nice -n 19 cargo build -q --profile profiling -p rerail-python-lean --example compare_strata
x="${CARGO_TARGET_DIR:-../target}/profiling/examples/compare_strata"
if command -v otool >/dev/null; then dis() { otool -tV "$x"; }; else dis() { objdump -d --no-show-raw-insn "$x"; }; fi
asm="$(mktemp)"
trap 'rm -f "$asm"' EXIT
dis > "$asm"
fail=0
for f in 9block_map 10then_block; do
    body="$(awk -v f="middle$f" '!on && index($0, f) && /:$/ {on=1; next} on && /^[^ \t].*:$/ {exit} on' "$asm")"
    n="$(grep -c . <<< "$body" || true)"
    jumps="$(grep -cE '\s(b\.[a-z]+|cbn?z|tbn?z|j[a-z]+|bl|blr|br|call[a-z]*)\s' <<< "$body" || true)"
    echo "$f: $n instructions, $jumps conditional branches or calls"
    if [ "$n" -lt 100 ] || [ "$jumps" -ne 0 ]; then fail=1; fi
done
exit "$fail"
