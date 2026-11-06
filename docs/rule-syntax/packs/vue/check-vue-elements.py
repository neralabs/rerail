#!/usr/bin/env python3
"""Checks lib/vue/elements.rr: its rows rebuild the JSON's lists exactly (order included), and, given the plugin's
checkout (--plugin DIR or $ESLINT_PLUGIN_VUE), equal the plugin's own lists as sets."""
import importlib.util, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-vue-elements.py"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)

rows = {}
for rel, x in re.findall(r'(v\w+)\("([^"]*)"\)\.', open(gen.OUT).read()):
    rows.setdefault(rel, []).append(x)
want = dict(zip(("vhtml_name", "vsvg_name", "vmath_name", "vsvg_weird_attr"), gen.lists()))
bad = [r for r in want if rows.get(r) != want[r]]
print("round trip:", "ok" if not bad else f"differs in {bad}")
plugin = sys.argv[sys.argv.index("--plugin") + 1] if "--plugin" in sys.argv else os.environ.get("ESLINT_PLUGIN_VUE")
if plugin:
    u = os.path.join(plugin, "lib", "utils")
    for rel, f in (("vhtml_name", "html-elements.json"), ("vsvg_name", "svg-elements.json"),
                   ("vmath_name", "math-elements.json"), ("vsvg_weird_attr", "svg-attributes-weird-case.json")):
        same = set(json.load(open(os.path.join(u, f)))) == set(rows[rel])
        print(f"plugin {f}:", "ok" if same else "differs")
        bad += [] if same else [rel]
sys.exit(1 if bad else 0)
