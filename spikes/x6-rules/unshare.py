#!/usr/bin/env python3
"""Today's shape, for comparison: every rule carries its own copy of every fact it reads, as the
original policies carry their own inputs and stage texts. Each rule's facts are copied under
`<fact>_<alias>`; a copied matcher gains the conjunct `not regex ^x6u<alias>$` (same rows, distinct
evidence, so the compiler does not share it); a copied membership fact reads the rule's copies.
Linking the result (`link.py --mode merged`) gives one document where nothing is shared between
rules except the clause-level linker (`references`), which the compiler builds once per clause.

usage: unshare.py --facts F[,F2] --rules R1,R2 --out-dir D   (prints "FACTS_FILE RULES")"""
import argparse
import json
import re
from pathlib import Path

import link


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--facts", required=True)
    ap.add_argument("--rules", required=True)
    ap.add_argument("--out-dir", required=True)
    a = ap.parse_args()
    facts, define, scope = link.load_facts(a.facts.split(","))
    out = Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    copies, files = {}, []
    for p in a.rules.split(","):
        r = json.loads(Path(p).read_text())
        al = r.get("alias", r["id"])
        names = link.reads(r, facts)
        ren = lambda q: re.sub(r"\b(FROM|JOIN)(\s+)([a-z0-9_]+)\b", lambda m: m[1] + m[2] + (f"{m[3]}_{al}" if m[3] in names else m[3]), q)
        for f in names:
            d = json.loads(json.dumps(facts[f]))
            if d["over"] == "content":
                rule = d["define"]["match"]["query"]["ast"]["rule"]
                guard = {"not": {"regex": f"^x6u{al}$"}}
                d["define"]["match"]["query"]["ast"]["rule"] = {**rule, "all": rule["all"] + [guard]} if "all" in rule else {"all": [rule, guard]}
            else:
                d["define"]["sql"] = link.mapq(d["define"]["sql"], ren)
            copies[f"{f}_{al}"] = d
        r["stages"] = [{**s, "query": link.mapq(s["query"], ren)} if "query" in s else s for s in r["stages"]]
        r["query"] = link.mapq(r["query"], ren)
        q = out / f"{al}.json"
        q.write_text(json.dumps(r) + "\n")
        files.append(str(q))
    ff = out / "facts.unshared.json"
    ff.write_text(json.dumps({"scope": scope, "define": define, "facts": copies}) + "\n")
    print(ff, ",".join(files))


if __name__ == "__main__":
    main()
