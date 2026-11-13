#!/bin/sh
# Regenerate docs/rule-syntax/packs/python-main/ (main's 537 Python JSON rules as engine .rr rules) and
# reader-check it.
#
#   ENGINE=<rerail-engine binary> [PACK_RULES=<main's pack rules dir>] [RERAIL_RR_LIB=<engine lib dir>] \
#       docs/rule-syntax/site/regen_main_pack.sh [ids file]
#
# PACK_RULES holds main's <id>.json files (default: the pack's pinned copy, packs/python-main/source/rules, main's
# Python pack 0.1.8); RERAIL_RR_LIB defaults to the repo's docs/rule-syntax/lib. The ids file (default: every JSON there) names the rules to
# translate. Each rule goes through translate_rules.py (translate.py, then engine_dialect.py); a hand-written
# overrides/<id>.rr then replaces the generated file. The reader check's verdicts go to reader-check.txt
# ("<status> <id>", and the first line of the error), sorted by id.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out="$here/../packs/python-main"
: "${PACK_RULES:=$out/source/rules}"
: "${ENGINE:?set ENGINE to the rerail-engine binary}"
: "${RERAIL_RR_LIB:=$here/../lib}"
export RERAIL_RR_LIB
ids=${1:--}

# Old generated files move to a trash dir, so a rule dropped from the id list does not linger; overrides and the
# README stay. (Nothing is deleted: the trash dir is under /tmp.)
trash=$(mktemp -d /tmp/ai-trash.rerail-main-pack.XXXXXX)
find "$out" -maxdepth 1 -name '*.rr' -exec mv {} "$trash"/ \;
nice -n 19 python3 "$here/translate_rules.py" "$PACK_RULES" "$ids" "$out"
for o in "$out"/overrides/*.rr; do
    [ -e "$o" ] || continue
    id=$(basename "$o" .rr)
    if [ -e "$out/$id.rr" ]; then cp "$o" "$out/$id.rr"; fi
done

check="$trash/check.json"
ls "$out"/*.rr | xargs nice -n 19 "$ENGINE" check-rr > "$check" 2>/dev/null || true
python3 - "$check" "$out/reader-check.txt" <<'EOF'
import json, os, sys
rows = json.load(open(sys.argv[1]))
lines = []
for r in rows:
    rid = os.path.basename(r["path"])[:-3]
    err = (r.get("error") or "").strip().splitlines()
    msg = ""
    if err:
        msg = err[1] if len(err) > 1 and err[0].endswith(":") else err[0]
        msg = msg.replace(os.path.dirname(os.path.abspath(r["path"])) + "/", "")
    lines.append(f"{r['status']} {rid}" + (f"  {msg[:200]}" if msg else ""))
lines.sort(key=lambda l: l.split()[1])
open(sys.argv[2], "w").write("\n".join(lines) + "\n")
ok = sum(1 for r in rows if r["status"] == "reader-passed")
print(f"{len(rows)} checked, {ok} reader-passed, {len(rows) - ok} reader-failed")
EOF
