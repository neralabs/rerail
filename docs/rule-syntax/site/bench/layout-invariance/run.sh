#!/bin/bash
# The layout-invariance gate (decision 0094 §8, t22's step 0) on corpus0's `conform` package (95 files): calls_cx cold,
# warm, the move warm on the same store, and the moved layout cold on a fresh store; then each layout's answers, its
# nodes placed by line and the moved layout's lines mapped back. Each run is capped at 60 s.
#
# Prepare the inputs here, from the shared data folder's corpus0 (bench-steps.sh's):
#   mkdir -p tmp/li/orig && cp -R "$RERAIL_SHARED/bench/step-vs-main/corpus0/conform" tmp/li/orig/
#   python3 -I docs/rule-syntax/site/bench/layout-invariance/move.py tmp/li/orig/ tmp/li/moved 8
# then run it from native/, here or on a farm Mac with the inputs sent:
#   scripts/dev/test-remote --remote-only --os macos --tree tmp/li -- \
#     bash ../docs/rule-syntax/site/bench/layout-invariance/run.sh ../tmp/li
set -u
T=$(cd "${CARGO_TARGET_DIR:-../target}" && pwd)
LI=$(cd "${1:-../tmp/li}" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
W=${TMPDIR:-/tmp}/oscar_li
rm -rf "$W"; mkdir -p -m 700 "$W"; chmod 700 "$W"
nice -n 19 cargo build --profile profiling --bin rerail-engine > "$W/build.log" 2>&1 || { echo "build failed"; grep -E '^error' -A5 "$W/build.log" | head -20; exit 1; }
E=$T/profiling/rerail-engine
# calls_cx, and the span of each node its answers name (as the oracle test's `oracle_jedi_at`), to compare layouts.
P=$W/calls_cx_at.rerail
cat src/code_rule_dsl/typing/programs/calls_cx.rerail > "$P"
cat >> "$P" <<'RR'

.decl answer_at(n: node, s: number)
answer_at(n, s) :- call_target(n, _), node(_, n, _, s, _).
answer_at(f, s) :- call_target(_, f), node(_, f, _, s, _).
answer_at(n, s) :- call_open(n), node(_, n, _, s, _).
answer_at(n, s) :- type_of(n, _), node(_, n, _, s, _).
.output answer_at
RR

run() { # <name> <corpus> <store>
  local t0=$(date +%s)
  timeout 60 env RERAIL_TAPE_CACHE="$3" RERAIL_COSTS="$W/costs.sqlite" nice -n 19 /usr/bin/time -l "$E" logic "$P" "$2" "$W/out-$1" > "$W/$1.log" 2>&1
  local rc=$?
  local cpu=$(awk '/ real /{c=$3+$5} END {printf "%.1f", c}' "$W/$1.log")
  local rss=$(awk '/maximum resident/{printf "%.0f", $1/1048576}' "$W/$1.log")
  echo "== $1: rc=$rc wall=$(( $(date +%s) - t0 ))s cpu=${cpu}s rss=${rss}MB"
  grep -E -i 'component|memo|stratum|strata|derived|found|kept|files' "$W/$1.log" | grep -v -E '^\s*[0-9.]+ +(real|user|sys)' | head -12
  return $rc
}

mkdir -p "$W/small"; cp -R "$LI/orig/conform/bootstrap_families" "$W/small/"
run small "$W/small" "$W/small.sqlite" || { tail -20 "$W/small.log"; exit 1; }
cp -R "$LI/orig" "$W/corpus"
run cold "$W/corpus" "$W/store.sqlite" || { tail -20 "$W/cold.log"; exit 1; }
run warm "$W/corpus" "$W/store.sqlite" || exit 1
# The move, in place: 8 files edited, 8 new.
cp -R "$LI/moved/." "$W/corpus/"
run moved "$W/corpus" "$W/store.sqlite" || exit 1
run moved-cold "$W/corpus" "$W/fresh.sqlite" || exit 1
for r in cold moved moved-cold; do RERAIL_COSTS="$W/costs.sqlite" RERAIL_TAPE_CACHE="$W/store.sqlite" "$E" csv "$W/out-$r" "$W/csv-$r" 2>&1 | tail -2; done
for r in moved moved-cold; do
  echo "-- answers, cold against $r:"
  python3 -I "$HERE/compare.py" "$LI/move-map.tsv" "$LI/orig" "$W/corpus" "$W/csv-cold" "$W/csv-$r" call_target call_open type_of
done
echo "W=$W"
