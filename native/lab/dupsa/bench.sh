#!/bin/zsh
# Speed: Jeff's rule (his engine, profiling build, fresh store each round) against dupsa (release), on his tree.
# usage: zsh tmp/dup-bench/bench.sh <label> <command...>   (run from the worktree root)
# Each round: a fresh state dir as cwd (so a fresh store), its own cost and image stores; /usr/bin/time -l.
# Prints one line a round: label, round, wall, user, sys, instructions, cycles, peak MB.
set -u
label=$1; shift
here=$PWD
for round in 1 2; do
  dir=$(mktemp -d $here/tmp/dup-bench/run.XXXXXX)
  ( cd $dir && RERAIL_COSTS=$dir/costs.sqlite RERAIL_IMAGE_STORE=$dir/images.sqlite \
      zsh $here/tmp/capped.sh 4000 300 nice -n 19 /usr/bin/time -l "$@" >$dir/stdout 2>$dir/stderr )
  awk -v l=$label -v r=$round '
    /real/ && /user/ { wall=$1; user=$3; sys=$5 }
    /instructions retired/ { ins=$1 }
    /cycles elapsed/ { cyc=$1 }
    /peak memory footprint/ { peak=$1 }
    END { printf "%s\tround %d\twall %.2f s\tuser %.2f s\tsys %.2f s\t%.1f G instr\t%.1f G cycles\tpeak %.0f MB\n", l, r, wall, user, sys, ins/1e9, cyc/1e9, peak/1e6 }
  ' $dir/stderr
  grep -E 'KILLED|panicked|error' $dir/stderr | head -3
  echo $dir
done
