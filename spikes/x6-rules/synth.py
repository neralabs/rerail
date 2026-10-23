#!/usr/bin/env python3
"""Synthetic repository rules for the scaling question: N rules of the shapes of the rewritten
rules, reading the same declared facts. Variant k of a shape is the shape's rule with its own
constants: one regex literal of its first private stage gains the alternative `^x6v<k>$`, and every
rule-private matcher gains the conjunct `not regex ^x6v<k>$` (no identifier or name of the corpus
has that spelling, so each variant finds what its shape finds; its plan, its matchers and its
stages are distinct, as a distinct rule's are: no sharing by digest or by evidence text).

usage: synth.py --mix dc:K,ef:K,... --out-dir DIR  (writes DIR/<alias>.json, prints the list)"""
import argparse
import json
from pathlib import Path

HERE = Path(__file__).parent
SHAPES = {  # alias -> (rule file, stage, literal, its variant)
    "dc": ("unreachable-internal-symbols", "dead_traits", "'validator$|overload$'", "'validator$|overload$|^x6v{k}$'"),
    "ef": ("exported-families", "traits", "'abstractmethod$'", "'abstractmethod$|^x6v{k}$'"),
    "rc": ("required-contracts", "frozen_decorated", "f.name='dataclass'", "f.name IN ('dataclass','x6v{k}')"),
    "sfp": ("state-flow-project-1", None, None, None),
    "qc": ("quality-conditional-1", None, None, None),
}


def perturb_matcher(ev, k):
    ev = json.loads(json.dumps(ev))
    rule = ev["evidence"]["query"]["ast"]["rule"]
    guard = {"not": {"regex": f"^x6v{k}$"}}
    if "all" in rule:
        rule["all"].append(guard)
    else:
        ev["evidence"]["query"]["ast"]["rule"] = {"all": [rule, guard]}
    return ev


def variant(shape, k):
    name, stage, lit, rep = SHAPES[shape]
    r = json.loads((HERE / "pack/rules" / f"{name}.json").read_text())
    r["id"], r["alias"] = f"{name}-v{k}", f"{shape}{k}"
    r["inputs"] = {n: perturb_matcher(v, k) for n, v in r.get("inputs", {}).items()}
    if stage is None:
        raise SystemExit(f"{shape}: choose a literal of its first private stage to vary (SHAPES)")
    s = next(s for s in r["stages"] if s["name"] == stage)
    assert isinstance(s["query"], str) and lit in s["query"], (shape, stage, lit)
    s["query"] = s["query"].replace(lit, rep.format(k=k), 1)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mix", required=True)
    ap.add_argument("--out-dir", required=True)
    a = ap.parse_args()
    out = Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    files = []
    for part in a.mix.split(","):
        shape, n = part.split(":")
        for k in range(int(n)):
            r = variant(shape, k)
            p = out / f"{r['alias']}.json"
            p.write_text(json.dumps(r) + "\n")
            files.append(str(p))
    print(",".join(files))


if __name__ == "__main__":
    main()
