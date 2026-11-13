"""Translate a pack's JSON policies into engine .rr rules, end to end.

    python3 translate_rules.py <pack rules dir> <id list file, or -> <out dir>

Each `<id>.json` under the rules dir goes through translate.translate (the display translation) and then
engine_dialect.engine_text (the engine's dialect), into `<out dir>/<id>.rr`. With `-`, every JSON in the dir.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import translate                          # noqa: E402
from engine_dialect import engine_text    # noqa: E402


def main(rules, ids, out):
    os.makedirs(out, exist_ok=True)
    names = (sorted(f[:-5] for f in os.listdir(rules) if f.endswith(".json")) if ids == "-"
             else open(ids).read().split())
    failed = 0
    for rid in names:
        doc = json.load(open(os.path.join(rules, rid + ".json")))
        try:
            rr = translate.translate(doc)[0]
        except Exception as e:     # a rule the translator cannot take: reported, the rest go on
            failed += 1
            print("FAIL", rid, type(e).__name__, e, file=sys.stderr)
            continue
        with open(os.path.join(out, rid + ".rr"), "w") as f:
            f.write(engine_text(rr, rid))
    print(f"{len(names) - failed} translated, {failed} failed")


if __name__ == "__main__":
    main(*sys.argv[1:4])
