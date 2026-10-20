#!/usr/bin/env bash
# The SQL engine's correctness gate (`just rerail-sql-gate`), on the `loop` profile in target-gate/:
#
#   rerail-sql-gate.sh            build, then every `sql::` test, with `replayed_cases_agree` on
#                                 native/src/code_rule_dsl/sql/tests/replay.jsonl and the differential at
#                                 the gate's case count (`cases` below), which kills every seeded mutant;
#   rerail-sql-gate.sh --mutants  build, then the replay and the differential alone on the clean build and
#                                 under each seeded mutant (`RERAIL_SQL_MUTANT`, `sql::mutant`): the clean
#                                 build must pass both, and each mutant fail both. Prints the kill matrix.
#
# RERAIL_SQL_GATE_CASES overrides the case count; CARGO_TARGET_DIR the build directory.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
# The smallest differential that kills all 21 mutants at the seed below: mutant 5's first killing case is
# case 1238, the last of the mutants' first kills; mutant 20 dies in the differential's own following
# cases (`following_cases`), which every run draws whatever the case count. A generator change moves
# it: re-measure with RERAIL_SQL_DIFF_KEEP under each mutant, and re-check with --mutants.
cases=${RERAIL_SQL_GATE_CASES:-1239}
seed=0x5e7c0de
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target-gate}

mode=gate
case "${1:-}" in
"") ;;
--mutants) mode=mutants ;;
*)
    echo "usage: rerail-sql-gate.sh [--mutants]" >&2
    exit 2
    ;;
esac

started=$SECONDS
build_log=$(mktemp)
trap 'rm -f "$build_log"' EXIT
if ! cargo test --profile loop --manifest-path native/Cargo.toml --lib --no-run 2>"$build_log"; then
    cat "$build_log" >&2
    exit 1
fi
binary=$(sed -n 's/^ *Executable unittests src\/lib.rs (\(.*\))$/\1/p' "$build_log")
[[ $binary == /* ]] || binary="$root/$binary"
echo "gate: built in $((SECONDS - started))s: $binary"

# A test run as `cargo test` runs it: from the package directory.
run() {
    (cd native && RERAIL_SQL_DIFF_CASES=$cases RERAIL_SQL_DIFF_SEED=$seed "$binary" "$@")
}

if [[ $mode == gate ]]; then
    run sql::
    echo "gate: passed in $((SECONDS - started))s ($cases differential cases)"
    exit 0
fi

replay=code_rule_dsl::sql::tests::differential::replayed_cases_agree
differential=code_rule_dsl::sql::tests::differential::generated_rules_agree_with_the_native_matcher
# The seeded mutants: the numbered list of `sql::mutant`'s documentation.
mutants=$(grep -cE '^/// [0-9]+\. ' native/src/code_rule_dsl/sql/mod.rs)
# Whether `test` passes under mutant `n` (0: none).
passes() {
    RERAIL_SQL_MUTANT=$1 run "$2" --exact >/dev/null 2>&1
}
failures=0
printf 'gate: %-8s %-8s %-12s\n' mutant replay "differential($cases)"
for n in 0 $(seq 1 "$mutants"); do
    row=()
    for test in "$replay" "$differential"; do
        if passes "$n" "$test"; then
            row+=("$([[ $n == 0 ]] && echo pass || echo SURVIVED)")
            [[ $n == 0 ]] || failures=$((failures + 1))
        else
            row+=("$([[ $n == 0 ]] && echo FAIL || echo killed)")
            [[ $n != 0 ]] || failures=$((failures + 1))
        fi
    done
    printf 'gate: %-8s %-8s %-12s\n' "$([[ $n == 0 ]] && echo clean || echo "$n")" "${row[0]}" "${row[1]}"
done
if ((failures > 0)); then
    echo "gate: $failures checks wrong (a surviving mutant or a failing clean build), $((SECONDS - started))s" >&2
    exit 1
fi
echo "gate: the clean build passes and all $mutants mutants are killed by both, $((SECONDS - started))s"
