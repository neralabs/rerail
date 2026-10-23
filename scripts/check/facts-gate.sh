#!/usr/bin/env bash
# The facts engine's gate (`just facts-gate`, Phase 2 deliverable 5 and G2.6; T2 of docs/facts/LOOP.md). One command:
#
#   facts-gate.sh             build (release, `mutants` feature, the tree's own target/release), then the
#                             parity and exactness checks on the clean build;
#   facts-gate.sh --mutants   the same, then every seeded mutant (`FACTS_MUTANT=<n>`) against the checks
#                             of its family: the clean build must pass every check and every mutant must
#                             be killed by at least one. Prints the kill matrix.
#
# Options: --only <check,...> runs a subset of the checks; --no-build reuses the last release build;
# --full replays all 1,111 edit events (default: the first 300); --run-dir <dir> overrides
# ~/rerail-bench/facts/phase-2/s5-gate/runs/<timestamp>-gate. Exit 0: passed, 1: a check failed or a
# mutant survived, 2: usage. Output format and checks: scripts/check/facts-gate.py and
# docs/facts/reports/phase-2-s5-gate.md § Gate.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
exec python3 "$root/scripts/check/facts-gate.py" "$@"
