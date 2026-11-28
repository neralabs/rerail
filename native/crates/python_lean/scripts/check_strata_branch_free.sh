#!/usr/bin/env bash
# The strata's block code has no branch (decision 0698, items 100 and 101): pass 1's block map and its composition
# (pass 3's too) compile to no conditional branch and no call, and stratum 1's block loop to its loop's own two (the
# test of an empty range and the back edge). Builds the comparison example (profiling) and reads its machine
# code. Run from native/. Needs otool (macOS) or objdump.
set -euo pipefail
nice -n 19 cargo build -q --profile profiling -p rerail-python-lean --example compare_strata
x="${CARGO_TARGET_DIR:-../target}/profiling/examples/compare_strata"
if command -v otool >/dev/null; then dis() { otool -tV "$x"; }; else dis() { objdump -d --no-show-raw-insn "$x"; }; fi
asm="$(mktemp)"
trap 'rm -f "$asm"' EXIT
dis > "$asm"
fail=0
# Each function: a name it holds (two, for a closure: the function and its part), and the branches it may have.
for spec in "middle9block_map::0" "middle10then_block::0" "classes_overKb0:fold:2" "middle10walk_blockKb0_::0" "middle10walk_blockKb1_::0" "middle9then_walk::0"; do
    IFS=: read -r f g allowed <<< "$spec"
    body="$(awk -v f="$f" -v g="$g" '!on && index($0, f) && index($0, g) && /:$/ {on=1; next} on && /^[^ \t].*:$/ {exit} on' "$asm")"
    n="$(grep -c . <<< "$body" || true)"
    jumps="$(grep -cE '\s(b\.[a-z]+|cbn?z|tbn?z|j[a-z]+|bl|blr|br|call[a-z]*)\s' <<< "$body" || true)"
    echo "$f: $n instructions, $jumps conditional branches or calls (at most $allowed)"
    if [ "$n" -lt 100 ] || [ "$jumps" -gt "$allowed" ]; then fail=1; fi
done
exit "$fail"
