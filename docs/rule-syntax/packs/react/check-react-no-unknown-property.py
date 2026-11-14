"""Check that react-no-unknown-property.props.rr says what the JSON says: rebuild each of the JSON's name sets from
the facts and diff them. Prints `ok`, or the differences. Run: python3 check-react-no-unknown-property.py"""
import json, os, re, sys, collections
sys.dont_write_bytecode = True

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
gen = __import__("gen-react-no-unknown-property")

facts = collections.defaultdict(list)
for rel, args in re.findall(r'(\w+)\(((?:"(?:[^"\\]|\\.)*"(?:, )?)+)\)\.', open(os.path.join(HERE, "react-no-unknown-property.props.rr")).read()):
    facts[rel].append(tuple(re.findall(r'"((?:[^"\\]|\\.)*)"', args)))

d = json.load(open(gen.JSON))
c = {x["id"]: x for x in d["clauses"]}
rule = lambda k: c[k]["forbid"]["ast"]["rule"]
bad = []
def same(what, want, got):
    if set(want) != set(got):
        bad.append(f"{what}: JSON only {sorted(set(want) - set(got))}, table only {sorted(set(got) - set(want))}")

only_on = collections.defaultdict(set)
for x, tag in facts["only_on"]:
    only_on[x].add(tag)
for k in c:
    if k.startswith("wrong-element-"):
        r = rule(k)
        for x, _ in gen.names(r["has"]["regex"]):
            same(f"{k} tags of {x}", [t for t, _ in gen.names(r["inside"]["not"]["has"]["regex"])], only_on[x])
same("wrong-element names", [x for k in c if k.startswith("wrong-element-") for x, _ in gen.names(rule(k)["has"]["regex"])],
     only_on)
same("html-or-svg-name", [x for x, _ in gen.names(rule("html-or-svg-name")["has"]["regex"])], [x for x, _ in facts["dom_attribute"]])
# The standard-name column: run the JSON's own transform steps (replace, then convert to camelCase) on each name.
def transform(steps, n):
    env = {"N": n}
    for var, step in steps.items():
        (op, a), = step.items()
        src = env[a["source"].lstrip("$")]
        if op == "replace":
            env[var] = re.sub(a["replace"], a["by"], src)
        elif op == "convert" and a["toCase"] == "camelCase" and a["separatedBy"] == ["dash"]:
            w = src.split("-")
            env[var] = w[0] + "".join(p[:1].upper() + p[1:] for p in w[1:])
        else:
            raise SystemExit(f"transform step {var}: {op} {a} is not modelled")
    return env[var]
steps = c["html-or-svg-name"]["forbid"]["ast"]["transform"]
same("html-or-svg-name, standard names", [(x, transform(steps, x)) for x, _ in facts["dom_attribute"]],
     facts["dom_attribute"])
lists = [gen.names(p["not"]["has"]["regex"]) for p in rule("unknown")["all"]]
same("unknown, exact names", [x for x, _ in max((l for l in lists if not any(i for _, i in l)), key=len)],
     [x for x, in facts["dom_property"]])
same("unknown, any-case names", [x for l in lists if l and all(i for _, i in l) and len(l) < 20 for x, _ in l],
     [x for x, in facts["any_case"]])
# The case-insensitive list is the exact one lower-cased, plus rendering-intent: eslint-plugin-react lists it as a DOM
# property too, but its SVG attribute entry (renderingIntent) is looked up first, so the JSON keeps it apart.
same("wrong-case names, lower-cased", [x for x, _ in gen.names(rule("wrong-case")["has"]["regex"])],
     [x.lower() for x, in facts["dom_property"]] + ["rendering-intent"])
aria = c["shared"]["define"]["utils"]["dom-attribute"]["all"][1]["not"]["has"]["regex"]
same("aria", [x for x, _ in gen.names(aria)], [x for x, in facts["aria_property"]])
print("\n".join(bad) if bad else "ok")
