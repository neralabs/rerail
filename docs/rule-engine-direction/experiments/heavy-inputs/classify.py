"""Manual target classification of the 52 relational clauses (read from the clause SQL; see report).
a = fully in IR/transducer; a+F = IR plus SIDE reads of a per-document fact family; a+c = IR plus small cross-file SQL tail; b+c = cross-file over fact families/narrow exports.
Usage: python3 -I classify.py CLAUSES_TXT"""
import sys, collections
M = {
 "authority-sites": ("a", "E8 opt.; allowlist qualnames compiled to inside-chains"),
 "direct-execution-lines": ("a", "dedupe+path filters -> forbid"),
 "unreachable-internal-symbols": ("b+c", "fact_def/import/ref/class_base + reachability"),
 "legacy-execution-graphs": ("a", "E15 keyed FIX (eager reach), FIX (receiver tokens), E16, transforms"),
 "exported-families": ("b+c", "traits per symbol in IR -> narrow t_export; family grouping via re-export closure"),
 "executor-provenance": ("a", "E15 scoped keyed FIX (name-binding propagation)"),
 "quality-contracts-1": ("a+F", "E8 SIDE over fact_scope/fact_binding (shadowed imports)"),
 "quality-contracts-2": ("a", "name/attr tests + filePath"),
 "quality-contracts-3": ("a", "E4 COUNT per allowlisted (class,function) limit"),
 "quality-local-taxonomy": ("a", "E4 COUNT from module ancestor"),
 "quality-project-taxonomy": ("b+c", "repository count >= 2 (budget over t_match)"),
 "quality-conditional-1": ("a+c", "E17 (always_raises), E15/E16; class_base closure + guard in SQL"),
 "state-signatures-1": ("a", "E15 keyed FIX (bad_configs by class name)"),
 "state-flow-1": ("a+c", "E15 scoped keyed FIX taint; cross-file return-kind summaries via E7 seed"),
 "state-projection-recovery-2": ("a", "E15; string keys inline; allowlist inline"),
 "state-flow-project-1": ("b+c", "cross-file data flow over per-function summary exports"),
 "runtime-reference-summaries": ("a", "E15 scoped keyed FIX (locals by name)"),
 "required-contracts": ("a+c", "NOT_EXISTS per required (path,name); missing-file existence in SQL"),
}
for i in range(1, 7): M[f"step-contracts-{i}"] = ("a", "E15 (name joins, keyed FIX), E16, E4")
cnt = collections.Counter(); sep = collections.Counter()
for line in open(sys.argv[1]):
    p = line.split(); clause = p[1]
    t = M.get(clause, ("a", "category A dedupe -> forbid + key"))[0]
    cnt[t] += 1; sep[t] += float([x for x in p if x.startswith("sep=")][0][4:].rstrip("s") or 0) if any(x.startswith("sep=") for x in p) else 0
print(dict(cnt), "total", sum(cnt.values())); print({k: round(v, 1) for k, v in sep.items()})
