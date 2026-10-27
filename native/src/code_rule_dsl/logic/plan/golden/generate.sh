#!/bin/bash
# Regenerates the planner goldens from Typing-support's spike (T), read-only:
#   generate.sh <T spikes/datalog-engines> <CPython 3.13 Lib/> <work dir>
# Facts: T's exporter over json/, ftplib, shlex, fnmatch and glob, then stats.py; kfreq, efreq and sqlite_stat4
# (unused by rdl.py) dropped. Goldens: rdl.py sql (no RDL_NATIVE) on the dump itself, its plan.json's "rules" as
# head<TAB>anchor<TAB>walks<TAB>work lines, and its output CSVs.
set -euo pipefail
T=$1 LIB=$2 W=$3 G=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$W/corpus" && cp -R "$LIB"/{json,ftplib.py,shlex.py,fnmatch.py,glob.py} "$W/corpus/"
cd "$W" && "$T/exporter/target/release/dl-export" corpus facts
python3 -I -B "$T/stats.py" build facts/facts.sqlite
sqlite3 facts/facts.sqlite "DROP TABLE kfreq; DROP TABLE efreq; DROP TABLE sqlite_stat4; VACUUM;"
sqlite3 facts/facts.sqlite .dump > facts.sql && zstd -19 -q -f facts.sql -o "$G/facts.sql.zst"
sqlite3 dumped.sqlite < facts.sql
for p in workload workload_demand calls; do
  mkdir -p "out/$p" "$G/$p"
  python3 -I -B "$T/rdl/rdl.py" sql "$T/rdl/$p.rdl" dumped.sqlite "out/$p"
  cp out/$p/*.csv "$G/$p/"
  python3 -I -c 'import json, sys
p = json.load(open(sys.argv[1]))
open(sys.argv[2], "w").write("".join(f"{r[\"head\"]}\t{r[\"anchor\"]}\t{json.dumps(r[\"walks\"])}\t{r[\"work\"]}\n" for r in p["rules"]))' "out/$p/plan.json" "$G/$p/rules.txt"
done
