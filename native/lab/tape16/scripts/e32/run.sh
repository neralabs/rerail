#!/bin/zsh
# E32 (t39 step 24): one pack run, outputs in tmp/e32/<tag>. run.sh <tag> <rules dir> <corpus> [MB] [s]
#   cold:  a fresh store, with the run's profile (RERAIL_PROFILE), strata trace and counts
#   warm:  the same store again, profiled
#   timed: the same store again, no profile (the profile's own writing costs ~25 s CPU after a cold run)
# Strict SQL always. RERAIL_NODE_SOURCES=eager by default (0174's oracle: the on-request answers lose findings when a
# reader is cross-file, 0174 §6); NS=request to run as the engine does by default. LIB: the rule library.
set -u
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
E=$PWD/target/profiling/rerail-engine
tag=$1; rules=$2; corpus=$3; mb=${4:-4000}; secs=${5:-120}
D=$PWD/tmp/e32/$tag
mkdir -p $D
ns=(); [ "${NS:-eager}" = eager ] && ns=(RERAIL_NODE_SOURCES=eager)
for step in cold warm timed; do
    mkdir -p $D/$step
    prof=(); [ $step != timed ] && ! { [ $step = cold ] && [ "${COLD_PROFILE:-1}" = 0 ]; } && prof=(RERAIL_PROFILE=$D/$step/profile RERAIL_STRATA_TRACE=$D/$step/strata.tsv RERAIL_STRATA_COUNTS=1)
    env $ns $prof RERAIL_SQL_STRICT=1 RERAIL_TAPE_CACHE=$D/store.sqlite RERAIL_RR_LIB=${LIB:-$PWD/docs/rule-syntax/lib} \
        zsh tmp/capped.sh $mb $secs /usr/bin/time -l $E logic $rules $corpus $D/$step/out --chunk 1000000 > $D/$step/stdout.txt 2> $D/$step/stderr.txt
    rc=$?
    echo "$tag $step rc=$rc $(awk '/ real /{printf "wall %.2f cpu %.2f", $1, $3+$5} /instructions retired/{printf " instr %.1fG", $1/1e9} /peak memory footprint/{printf " peak %.0f MB", $1/1048576}' $D/$step/stderr.txt) findings $( [ -f $D/$step/out/violation.csv ] && wc -l < $D/$step/out/violation.csv | tr -d ' ' || echo '?')"
done
