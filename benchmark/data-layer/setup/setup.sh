#!/usr/bin/env bash
# `just rerail-bench-setup`: lays out the SQL loop's benchmark home from repositories the user already has.
#
#   just rerail-bench-setup --data-layer <nera-data-layer clone> --nera-landing <nera-landing clone> \
#     [--home <dir>] [--addon <rerail_engine.node>]
#
# The home is --home, else RERAIL_BENCH_HOME, else ~/rerail-bench. Into it:
#   nera-data-layer-<commit>/  the corpus: a detached worktree of the nera-data-layer clone at loop-subset.json's commit
#                              (the clone's own checkout is not touched);
#   data-layer/policies/       the 12 Tempo ports converted to the current contract, beside their oracles
#                              (convert.mts, from the nera-landing benchmark commit below);
#   data-layer/conversion-report.json.
# It fetches only what the clones lack, builds the `loop` addon into target-loop/ when it is missing (the one
# `just rerail-sql-loop` builds; --addon names another for the conversion), and checks the result with verify.mts:
# every oracle file is the commit's, and every policy hashes to what loop-subset.json was selected with. A home that
# already checks out is left as it is, so a second run only checks.
set -euo pipefail

# The Tempo oracles and ports: nera-landing's benchmark branch, at the commit the loop's policies were converted from.
oracle_branch=Moti-Velz/tempo-data-layer-benchmark
oracle_commit=aa63f36b580d4c90b9b955c77db9066f0733eb38

usage() {
  echo "usage: just rerail-bench-setup --data-layer <nera-data-layer clone> --nera-landing <nera-landing clone> [--home <dir>] [--addon <rerail_engine.node>]" >&2
  exit 2
}
die() {
  echo "rerail-bench-setup: $*" >&2
  exit 1
}

data_layer=
nera_landing=
home=${RERAIL_BENCH_HOME:-$HOME/rerail-bench}
addon=
while [ $# -gt 0 ]; do
  case $1 in
    --data-layer) data_layer=${2:-}; shift 2 || usage ;;
    --nera-landing) nera_landing=${2:-}; shift 2 || usage ;;
    --home) home=${2:-}; shift 2 || usage ;;
    --addon) addon=${2:-}; shift 2 || usage ;;
    *) usage ;;
  esac
done
[ -n "$data_layer" ] && [ -n "$nera_landing" ] && [ -n "$home" ] || usage

rerail=$(git rev-parse --show-toplevel)
cd "$rerail"
git -C "$data_layer" rev-parse --git-dir >/dev/null 2>&1 || die "--data-layer $data_layer is not a git clone"
git -C "$nera_landing" rev-parse --git-dir >/dev/null 2>&1 || die "--nera-landing $nera_landing is not a git clone"
data_layer=$(cd "$data_layer" && pwd -P)
nera_landing=$(cd "$nera_landing" && pwd -P)
mkdir -p "$home/data-layer"
home=$(cd "$home" && pwd -P)

has_commit() { git -C "$1" cat-file -e "$2^{commit}" 2>/dev/null; }

# 1. The corpus: loop-subset.json's commit, in a detached worktree.
corpus_commit=$(node -p 'JSON.parse(require("node:fs").readFileSync("benchmark/data-layer/loop-subset.json", "utf8")).corpus.commit')
corpus="$home/nera-data-layer-${corpus_commit:0:8}"
if ! has_commit "$data_layer" "$corpus_commit"; then
  echo "corpus: fetching $corpus_commit into $data_layer"
  git -C "$data_layer" fetch origin "$corpus_commit" || git -C "$data_layer" fetch origin
  has_commit "$data_layer" "$corpus_commit" || die "$data_layer's origin has no commit $corpus_commit"
fi
if [ -e "$corpus" ]; then
  head=$(git -C "$corpus" rev-parse HEAD 2>/dev/null) || die "$corpus exists and is not a git checkout"
  [ "$head" = "$corpus_commit" ] || die "$corpus is at $head, not $corpus_commit"
  [ -z "$(git -C "$corpus" status --porcelain --untracked-files=all)" ] || die "$corpus has local changes; the loop reads the working tree"
  echo "corpus: $corpus at ${corpus_commit:0:9}, clean"
else
  git -C "$data_layer" worktree add --detach "$corpus" "$corpus_commit"
  echo "corpus: created $corpus at ${corpus_commit:0:9}"
fi

# 2. The oracle commit.
if ! has_commit "$nera_landing" "$oracle_commit"; then
  echo "oracles: fetching $oracle_branch into $nera_landing"
  git -C "$nera_landing" fetch origin "$oracle_branch"
  has_commit "$nera_landing" "$oracle_commit" || die "$nera_landing's origin/$oracle_branch does not contain $oracle_commit"
fi
tip=$(git -C "$nera_landing" rev-parse -q --verify "origin/$oracle_branch^{commit}" || true)
if [ -n "$tip" ] && [ "$tip" != "$oracle_commit" ]; then
  echo "oracles: origin/$oracle_branch is at ${tip:0:9}; converting ${oracle_commit:0:9}, the commit loop-subset.json's policies come from"
fi

# 3. The addon the conversion compiles each policy with.
[ -d node_modules ] || npm ci
platform=$(node -p 'process.platform + "-" + process.arch')
loop_addon="$rerail/target-loop/addon/rerail_engine.$platform.node"
if [ -z "$addon" ]; then
  addon=$loop_addon
  if [ ! -f "$addon" ]; then
    echo "addon: building the loop profile into target-loop/ (as \`just rerail-sql-loop\` does)"
    CARGO_TARGET_DIR="$rerail/target-loop" npx napi build --platform --profile loop --features unbounded-work --manifest-path native/Cargo.toml --output-dir target-loop/addon
  fi
fi
[ -f "$addon" ] || die "no addon at $addon"
addon=$(cd "$(dirname "$addon")" && pwd -P)/$(basename "$addon")

# 4. The policies: converted unless the home already checks out.
policies="$home/data-layer/policies"
verify() { node --import tsx benchmark/data-layer/setup/verify.mts --policies "$policies" --nera-landing "$nera_landing" --commit "$oracle_commit"; }
if check=$(verify); then
  echo "policies: $check"
else
  if [ -e "$policies" ]; then echo "policies: converting again ($(printf '%s\n' "$check" | tail -n 1))"; fi
  node --import tsx benchmark/data-layer/setup/convert.mts --rerail "$rerail" --addon "$addon" --nera-landing "$nera_landing" --ref "$oracle_commit" --out "$policies"
  if ! check=$(verify); then
    printf '%s\n' "$check" >&2
    die "the converted policies do not match loop-subset.json: a policy hash differs when this checkout's code-rule contract (src/engine/rules.ts) stores a rule differently from the one loop-subset.json was selected with, and the subset's coverage is then stale (\`just rerail-sql-loop-tool select\`)"
  fi
  echo "policies: $check"
fi

# 5. What to run next.
export_line=
[ "$home" = "$(cd "$HOME" && pwd -P)/rerail-bench" ] || export_line="export RERAIL_BENCH_HOME=$home"
cat <<EOF

The benchmark home $home is ready. From $rerail:
${export_line:+  $export_line
}  just rerail-sql-loop --clauses hasattr_async   # a small first run: builds the loop addon, measures native once (cached), reaches a VERDICT
  just rerail-sql-loop                           # the whole loop: every reporting clause, then the selected mutations
  just rerail-sql-gate                           # the SQL engine's correctness gate
To serialize timed runs with other measurements on this machine: RERAIL_BENCH_LOCK=benchmark/data-layer/setup/bench-lock just rerail-sql-loop
EOF
