"""Build the Rerail logic syntax page: out/index.html and out/data/*.json.

    python3 docs/rule-syntax/site/build.py [--dlre <Data-layer-rule-expressivity worktree>] [--ndl <nera-data-layer checkout>]
"""

import argparse
import html
import glob
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
RS = os.path.join(ROOT, "docs", "rule-syntax")
sys.path.insert(0, HERE)
import translate  # noqa: E402
import runpy  # noqa: E402

LIBS = [
    ("core", "native", "What the binary provides: tape relations, tree arithmetic, value builtins, repository readers."),
    ("python/names", "datalog", "Scopes, bindings, references and qualified names. LEGB and rebinding are lifted from Typing-support's calls.rdl."),
    ("python/imports", "datalog", "What each import binds and loads, the import-time module graph, and module exports."),
    ("python/calls", "datalog", "Call targets and call chains: calls.rdl's points-to resolution, three-valued through call_open."),
    ("python/flow", "datalog", "Statement order within a body. Tree arithmetic only."),
    ("tsx/names", "datalog", "TypeScript and JavaScript scopes after eslint-scope, import specifier resolution, exports and symbols across files."),
    ("tsx/jsx", "datalog", "JSX tags, components, and what renders under what."),
]
DATA_LAYER = [
    ("layer-violations", "layer-violations", "check_layer_violations.py"),
    ("public-api-triples", "public-api-triples", "lint_public_api_triples.py"),
    ("plan-scope-side-effects", "plan-scope-side-effects", "lint_plan_scope_side_effects.py"),
    ("missing-async-pairs", "missing-async-pairs", "lint_missing_async_pairs.py"),
    ("post-op-no-internal-dispatch", "post-op-no-internal-dispatch", "lint_post_op_no_internal_dispatch.py"),
    ("swallowed-exceptions", "swallowed-exceptions", "lint_swallowed_exceptions.py"),
]
NDL_COMMIT = "2b1504adc4291daaf080b2e932962b134da1a69a"
PR2539 = "origin/jeremy/serverplan-architecture-integration"


def read(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


def compact(obj):
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))


def description(rr):
    m = re.search(r'^\.description\s+"((?:[^"\\]|\\.)*)"', rr, re.M)
    return json.loads('"' + m.group(1) + '"') if m else ""


def git_show(repo, rev, path):
    try:
        return subprocess.run(["git", "-C", repo, "show", f"{rev}:{path}"], check=True, capture_output=True, text=True).stdout
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dlre", default=os.path.join(os.path.dirname(ROOT), "Data-layer-rule-expressivity"))
    ap.add_argument("--ndl", default=os.path.join(os.path.dirname(os.path.dirname(ROOT)), "nera-data-layer-ner1131-base-audit"))
    ap.add_argument("--out", default=os.path.join(HERE, "out"))
    a = ap.parse_args()
    runpy.run_path(os.path.join(HERE, "pages.py"))   # writes intro.html and syntax.html
    os.makedirs(os.path.join(a.out, "data"), exist_ok=True)

    libs = {}
    for mod, kind, blurb in LIBS:
        libs[mod] = {"kind": kind, "src": read(os.path.join(RS, "lib", mod + ".rr")), "blurb": blurb}
    n_native = sum(len(re.findall(r"^\.native\s", l["src"], re.M)) for l in libs.values())
    n_datalog = sum(len(re.findall(r"^\.(?:decl|edit)\s", l["src"], re.M)) for m, l in libs.items() if l["kind"] == "datalog")

    packs, total_json, total_rr, atom_rules, n_rules = [], 0, 0, 0, 0
    for pdir in sorted(glob.glob(os.path.join(ROOT, "packs", "*", "rules"))):
        name = pdir.split(os.sep)[-2]
        rules = []
        for f in sorted(glob.glob(os.path.join(pdir, "*.json"))):
            doc = json.loads(read(f))
            rr, u = translate.translate(doc)
            pretty = json.dumps(doc, indent=2, ensure_ascii=False)
            rules.append({"id": doc.get("id", os.path.basename(f)), "d": doc.get("description", ""), "m": doc.get("mode", "log"),
                          "j": compact(doc), "r": rr, "u": u, "jl": pretty.count("\n") + 1, "rl": rr.count("\n")})
            total_json += len(pretty)
            total_rr += len("\n".join(l for l in rr.split("\n") if not l.lstrip().startswith("//")))  # code only
            atom_rules += bool(u)
        if not rules:
            continue
        n_rules += len(rules)
        with open(os.path.join(a.out, "data", f"pack-{name}.json"), "w", encoding="utf-8") as fh:
            fh.write(compact(rules))
        packs.append({"name": name, "count": len(rules)})
    packs.sort(key=lambda p: (-p["count"], p["name"]))

    hand_meta, hand_data = [], {}
    router_json = json.loads(read(os.path.join(RS, "nested-routers.json")))
    b1, b2 = read(os.path.join(RS, "nested-routers.b1.rr")), read(os.path.join(RS, "nested-routers.b2.rr"))
    hand_data["router"] = {"json": compact(router_json), "gen": translate.translate(router_json)[0], "b1": b1, "b2": b2}
    hand_meta.append({"key": "router", "title": "no-nested-routers", "tags": ["worked example", "typescript"],
                      "summary": "A react-router router nested inside another. Today's JSON, its generated form, the same meaning written by hand, and a more accurate version that follows components across files.",
                      "views": [
                          {"label": "B2: more accurate", "panes": [{"title": "B2 .rr", "src": "b2", "lang": "rr"}, {"title": "B1 .rr: same meaning as the JSON", "src": "b1", "lang": "rr"}]},
                          {"label": "Today vs B1", "panes": [{"title": "Today: JSON", "src": "json", "lang": "json"}, {"title": "B1 .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Today vs generated", "panes": [{"title": "Today: JSON", "src": "json", "lang": "json"}, {"title": "Generated .rr", "src": "gen", "lang": "rr"}]},
                      ]})

    fli1, fli2 = read(os.path.join(RS, "function-level-imports.b1.rr")), read(os.path.join(RS, "function-level-imports.b2.rr"))
    yaml = git_show(a.ndl, PR2539, "scripts/lint_specs/lint_function_level_imports.yaml") or "# PR 2539 YAML not available in this checkout\n"
    py = git_show(a.ndl, NDL_COMMIT, "scripts/lint_function_level_imports.py") or "# oracle not available\n"
    hand_data["fli"] = {"yaml": yaml, "b1": fli1, "b2": fli2, "py": py}
    hand_meta.append({"key": "fli", "title": "function-level-imports", "tags": ["data-layer", "ratchet"],
                      "summary": "Classifies imports inside functions. B1 matches the Python linter; B2 classifies from evidence: proven import cycles, annotation-only use, ImportError guards.",
                      "views": [
                          {"label": "B2: from evidence", "panes": [{"title": "B2 .rr", "src": "b2", "lang": "rr"}, {"title": "B1 .rr: the Python linter's meaning", "src": "b1", "lang": "rr"}]},
                          {"label": "PR 2539 YAML vs B1", "panes": [{"title": "PR 2539 YAML (nera.linter/v1)", "src": "yaml", "lang": "yaml"}, {"title": "B1 .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Python oracle vs B1", "panes": [{"title": "scripts/lint_function_level_imports.py", "src": "py", "lang": "text"}, {"title": "B1 .rr", "src": "b1", "lang": "rr"}]},
                      ]})

    for key, slug, pyfile in DATA_LAYER:
        p = os.path.join(RS, "data-layer", slug + ".rr")
        if not os.path.exists(p):
            continue
        final = read(p)
        cand = os.path.join(a.dlre, "packs", "data-layer", "candidates", "rules", f"data-layer-{slug}.json")
        entry = {"final": final}
        views = [{"label": "Final form", "panes": [{"title": slug + ".rr", "src": "final", "lang": "rr"}]}]
        if os.path.exists(cand):
            doc = json.loads(read(cand))
            entry["json"] = compact(doc)
            entry["gen"] = translate.translate(doc)[0]
            views[0]["panes"].append({"title": "Today: candidate JSON port", "src": "json", "lang": "json"})
            views.append({"label": "Candidate JSON vs generated", "panes": [{"title": "Candidate JSON port", "src": "json", "lang": "json"}, {"title": "Generated .rr", "src": "gen", "lang": "rr"}]})
        src = git_show(a.ndl, NDL_COMMIT, "scripts/" + pyfile)
        if src:
            entry["py"] = src
            views.append({"label": "Python oracle vs final", "panes": [{"title": "scripts/" + pyfile, "src": "py", "lang": "text"}, {"title": slug + ".rr", "src": "final", "lang": "rr"}]})
        hand_data[key] = entry
        hand_meta.append({"key": key, "title": slug, "tags": ["data-layer"], "summary": description(final), "views": views})

    with open(os.path.join(a.out, "data", "hand.json"), "w", encoding="utf-8") as fh:
        fh.write(compact(hand_data))

    def code_lines(text):
        return sum(1 for l in text.split("\n") if l.strip() and not l.strip().startswith("//"))
    dl_py = sum(e["py"].count("\n") for k, e in hand_data.items() if k not in ("router", "fli") and "py" in e)
    dl_rr = sum(code_lines(e["final"]) for e in hand_data.values() if "final" in e)
    lib_lines = lambda fam: sum(code_lines(l["src"]) for m, l in libs.items() if m.startswith(fam))
    router_pretty = json.dumps(router_json, indent=2, ensure_ascii=False)
    stats = {"rules": f"{n_rules:,}",
             "router_json_lines": str(router_pretty.count("\n") + 1), "router_b1_code": str(code_lines(b1)),
             "dl_py_lines": f"{dl_py:,}", "dl_rr_code": f"{dl_rr:,}",
             "lib_py_lines": f"{lib_lines('python/'):,}", "lib_tsx_lines": f"{lib_lines('tsx/'):,}", "json_mb": f"{total_json / 1e6:.1f} MB", "rr_mb": f"{total_rr / 1e6:.1f} MB",
             "atom_rules": f"{atom_rules:,}", "lib_native": str(n_native), "lib_datalog": str(n_datalog)}

    def embed(obj):
        return compact(obj).replace("</", "<\\/")

    page = read(os.path.join(HERE, "page.html"))
    page = (page.replace("__LIBS__", embed(libs)).replace("__HAND__", embed(hand_meta)).replace("__PACKS__", embed(packs))
            .replace("__STATS__", embed(stats)).replace("__SYNTAX__", read(os.path.join(HERE, "syntax.html")))
            .replace("__INTRO__", read(os.path.join(HERE, "intro.html"))
                     .replace("@@router_json@@", html.escape(router_pretty)).replace("@@router_b1@@", html.escape(b1.rstrip("\n")))))
    with open(os.path.join(a.out, "index.html"), "w", encoding="utf-8") as fh:
        fh.write(page)
    print(json.dumps({"rules": n_rules, "packs": len(packs), "hand": len(hand_meta), **stats}))


if __name__ == "__main__":
    main()
