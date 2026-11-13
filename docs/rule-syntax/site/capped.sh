#!/bin/bash
# capped.sh <secs> <MB> <log> <cmd...>: runs cmd at nice 19 under /usr/bin/time, with RERAIL_SQL_STRICT=1; kills it
# past <secs> seconds or <MB> MB of resident memory. Prints one summary line; a kill is reported there
# ("KILLED: …"), not in the log (time writes the log).
secs=$1; mb=$2; log=$3; shift 3
RERAIL_SQL_STRICT=1 nice -n 19 /usr/bin/time -l "$@" > "$log" 2>&1 & tp=$!
sleep 0.5; pid=$(pgrep -P $tp | head -1)
start=$SECONDS; t=0; killed=""
while kill -0 $tp 2>/dev/null; do
  rss=$(ps -o rss= -p ${pid:-$tp} 2>/dev/null | tr -d ' '); rss=${rss:-0}
  if [ $((rss/1024)) -gt $mb ]; then kill -9 ${pid:-$tp}; killed="KILLED: rss $((rss/1024)) MB > $mb MB at ${t}s"; break; fi
  if [ $t -ge $secs ]; then kill -9 ${pid:-$tp}; killed="KILLED: time > ${secs}s"; break; fi
  sleep 0.5; t=$((SECONDS-start))
done
wait $tp 2>/dev/null; rc=$?
awk -v rc=$rc -v k="$killed" '/ real /{w=$1;c=$3+$5} /maximum resident/{p=$1/1048576} END{printf "rc=%d cpu %.1f s wall %.1f s peak %.0f MB %s\n", rc, c, w, p, k}' "$log"
