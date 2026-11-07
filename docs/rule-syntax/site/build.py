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
    ("python/names", "datalog", "Scopes, bindings, references and qualified names. Python's scope lookup and rebinding, adapted from an earlier Datalog prototype."),
    ("python/imports", "datalog", "What each import binds and loads, the import-time module graph, and module exports."),
    ("python/calls", "datalog", "Call targets and call chains: which function a call can reach, with unknown targets kept as unknown."),
    ("python/ast", "datalog", "Python as CPython's ast sees it: Name and context, frames with decorators inside the def, parameters by kind, assignments, constants, qualified names, and visit order."),
    ("python/flow", "datalog", "Statement order within a body, and blocks that always raise. Tree arithmetic only."),
    ("data-layer/signing", "datalog", "The data-layer repository's signed allowlists and ratchet baselines, verified in Datalog: the signed bytes rebuilt with join, checked with @ed25519_ok."),
    ("tsx/names", "datalog", "TypeScript and JavaScript scopes after eslint-scope, import specifier resolution, exports and symbols across files."),
    ("tsx/jsx", "datalog", "JSX tags, components, and what renders under what."),
    ("tsx/statements", "datalog", "Statement bodies, braces, dangling if/else, one-liners and ASI hazards, and declarations with their declarators: what curly and one-var need."),
    ("tsx/modules", "datalog", "Directive prologues and strict mode, source type, and import declarations as sort-imports sees them."),
    ("tsx/values", "datalog", "Which Node.js built-in or global API an expression denotes: require, imports, destructuring, globalThis."),
]
DATA_LAYER = [
    ("layer-violations", "layer-violations", "check_layer_violations.py"),
    ("public-api-triples", "public-api-triples", "lint_public_api_triples.py"),
    ("plan-scope-side-effects", "plan-scope-side-effects", "lint_plan_scope_side_effects.py"),
    ("missing-async-pairs", "missing-async-pairs", "lint_missing_async_pairs.py"),
    ("post-op-no-internal-dispatch", "post-op-no-internal-dispatch", "lint_post_op_no_internal_dispatch.py"),
    ("swallowed-exceptions", "swallowed-exceptions", "lint_swallowed_exceptions.py"),
    ("file-length", "file-length", "lint_file_length.py"),
    ("noqa-sec-ratchet", "noqa-sec-ratchet", "lint_noqa_sec_ratchet.py"),
    ("versioned-table-names", "versioned-table-names", "lint_versioned_table_name_manipulation.py"),
    ("test-budget", "test-budget", "check_test_budget.py"),
]
# The data-layer port's later batches (7-10), hand-written from the Python linters at NDL_COMMIT.
DATA_LAYER += [(s, s, f"lint_{s.replace('-', '_')}.py") for s in """
    authentication-plan-purity bootstrap-cache-health bootstrap-eager-resolve bootstrap-engine-invariants
    bootstrap-lazy-sql bounded-composed-graph-paths conditional-architecture connection-mode-split
    connection-service-architecture connection-structural-boundaries declared-fact-interrogation eager-resolve
    embedding-vector-orchestration executor-publication-repairs generated-index-sources grammar-mutation-invalidation
    graph-lazy-sql-plan graphql-plan-ownership history-serverplan-projection hnsw-architecture lazy-adapter-ownership
    lazy-architecture lazy-sql-surface legacy-callback-quarantine matching-architecture plan-composition-architecture
    plan-helpers-allowlist plan-optimizer-v2 post-op-side-effects publication-registry-writes pure-lazy-runtime-purity
    readthrough-authority-architecture revision-ancestry-invariants schema-architecture schema-bootstrap-contracts
    schema-introspection-data-paths unknown-contract-defaults""".split()]
DATA_LAYER += [("versioned-merged-identity", "versioned-merged-identity", "check_versioned_merged_identity.py"),
               ("cached-pure-lazy-allowlist", "cached-pure-lazy-allowlist", "verify_cached_pure_lazy_allowlist.py")]
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
    n_native = sum(len(re.findall(r"^\.(?:decl|edit)\s", l["src"], re.M)) for l in libs.values() if l["kind"] == "native")
    n_datalog = sum(len(re.findall(r"^\.(?:decl|edit)\s", l["src"], re.M)) for m, l in libs.items() if l["kind"] == "datalog")

    packs, total_json, total_rr, atom_rules, n_rules, ungrounded = [], 0, 0, 0, 0, 0
    hand_rules, hand_rr = 0, 0   # hand-written pack rules and their size, code only (tables excluded)
    for pdir in sorted(glob.glob(os.path.join(ROOT, "packs", "*", "rules"))):
        name = pdir.split(os.sep)[-2]
        rules = []
        for f in sorted(glob.glob(os.path.join(pdir, "*.json"))):
            doc = json.loads(read(f))
            rr, u, ug = translate.translate(doc)
            ungrounded += ug
            pretty = json.dumps(doc, indent=2, ensure_ascii=False)
            rid = doc.get("id", os.path.basename(f))
            entry = {"id": rid, "d": doc.get("description", ""), "m": doc.get("mode", "log"),
                     "j": compact(doc), "r": rr, "u": u, "jl": pretty.count("\n") + 1, "rl": rr.count("\n")}
            # The hand-written rule: docs/rule-syntax/packs/<pack>/<id>.rr, plus <id>.<part>.rr files (a fact table).
            hand = os.path.join(RS, "packs", name, rid + ".rr")
            if os.path.exists(hand):
                entry["h"] = read(hand)
                hand_rules += 1
                hand_rr += len("\n".join(l for l in entry["h"].split("\n") if not l.lstrip().startswith("//")))
                entry["hx"] = {os.path.basename(x)[len(rid) + 1:-3]: read(x)
                               for x in sorted(glob.glob(os.path.join(RS, "packs", name, rid + ".*.rr")))}
            rules.append(entry)
            total_json += len(pretty)
            total_rr += len("\n".join(l for l in rr.split("\n") if not l.lstrip().startswith("//")))  # code only
            atom_rules += bool(u)
        if not rules:
            continue
        n_rules += len(rules)
        with open(os.path.join(a.out, "data", f"pack-{name}.json"), "w", encoding="utf-8") as fh:
            fh.write(compact(rules))
        rules.sort(key=lambda r: "h" not in r)  # hand-written exemplars first, then the JSON's order
        packs.append({"name": name, "count": len(rules), "hand": sum("h" in r for r in rules)})
    packs.sort(key=lambda p: (-p["count"], p["name"]))

    hand_meta, hand_data = [], {}
    router_json = json.loads(read(os.path.join(RS, "nested-routers.json")))
    b1, b2 = read(os.path.join(RS, "nested-routers.b1.rr")), read(os.path.join(RS, "nested-routers.b2.rr"))
    hand_data["router"] = {"json": compact(router_json), "gen": translate.translate(router_json)[0], "b1": b1, "b2": b2}
    hand_meta.append({"key": "router", "title": "no-nested-routers", "tags": ["worked example", "typescript"],
                      "summary": "A react-router router nested inside another. Today's JSON, its generated form, the same meaning written by hand, and a more accurate version that follows components across files.",
                      "views": [
                          {"label": "More accurate", "panes": [{"title": "Accurate version .rr", "src": "b2", "lang": "rr"}, {"title": "Same meaning as the JSON .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Today vs .rr", "panes": [{"title": "Today: JSON", "src": "json", "lang": "json"}, {"title": "Same meaning .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Today vs generated", "panes": [{"title": "Today: JSON", "src": "json", "lang": "json"}, {"title": "Generated .rr", "src": "gen", "lang": "rr"}]},
                      ]})

    fli1, fli2 = read(os.path.join(RS, "function-level-imports.b1.rr")), read(os.path.join(RS, "function-level-imports.b2.rr"))
    yaml = git_show(a.ndl, PR2539, "scripts/lint_specs/lint_function_level_imports.yaml") or "# The earlier YAML port is not available in this checkout\n"
    py = git_show(a.ndl, NDL_COMMIT, "scripts/lint_function_level_imports.py") or "# The Python linter is not available in this checkout\n"
    hand_data["fli"] = {"yaml": yaml, "b1": fli1, "b2": fli2, "py": py}
    hand_meta.append({"key": "fli", "title": "function-level-imports", "tags": ["data-layer", "ratchet"],
                      "summary": "Classifies imports inside functions. One version matches today's Python linter exactly; the accurate version classifies from evidence: proven import cycles, annotation-only use, ImportError guards.",
                      "views": [
                          {"label": "From evidence", "panes": [{"title": "Accurate version .rr", "src": "b2", "lang": "rr"}, {"title": "Same meaning as the Python linter .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Earlier YAML attempt vs .rr", "panes": [{"title": "Earlier attempt: YAML with embedded SQL", "src": "yaml", "lang": "yaml"}, {"title": "Same meaning as the Python linter .rr", "src": "b1", "lang": "rr"}]},
                          {"label": "Python linter vs .rr", "panes": [{"title": "scripts/lint_function_level_imports.py", "src": "py", "lang": "text"}, {"title": "Same meaning as the Python linter .rr", "src": "b1", "lang": "rr"}]},
                      ]})

    for key, slug, pyfile in DATA_LAYER:
        p = os.path.join(RS, "data-layer", slug + ".rr")
        if not os.path.exists(p):
            continue
        final = read(p)
        cand = next((c for c in (os.path.join(a.dlre, "packs", "data-layer", d, "rules", f"data-layer-{slug}.json")
                                 for d in ("candidates", ".")) if os.path.exists(c)), "")
        entry = {"final": final}
        views = [{"label": "Final form", "panes": [{"title": slug + ".rr", "src": "final", "lang": "rr"}]}]
        if os.path.exists(cand):
            doc = json.loads(read(cand))
            entry["json"] = compact(doc)
            views[0]["panes"].append({"title": "Today: JSON port", "src": "json", "lang": "json"})
            try:   # the machine translator reads the early JSON format only; later ports get no generated view
                entry["gen"] = translate.translate(doc)[0]
                views.append({"label": "JSON port vs generated", "panes": [{"title": "JSON port in today's format", "src": "json", "lang": "json"}, {"title": "Generated .rr", "src": "gen", "lang": "rr"}]})
            except Exception:
                pass
        src = git_show(a.ndl, NDL_COMMIT, "scripts/" + pyfile)
        if src:
            entry["py"] = src
            views.append({"label": "Python linter vs .rr", "panes": [{"title": "scripts/" + pyfile, "src": "py", "lang": "text"}, {"title": slug + ".rr", "src": "final", "lang": "rr"}]})
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
    stats = {"rules": f"{n_rules:,}", "hand_rules": f"{hand_rules:,}", "hand_mb": f"{hand_rr / 1e6:.1f} MB",
             "router_json_lines": str(router_pretty.count("\n") + 1), "router_b1_code": str(code_lines(b1)),
             "dl_py_lines": f"{dl_py:,}", "dl_rr_code": f"{dl_rr:,}",
             "lib_py_lines": f"{lib_lines('python/'):,}", "lib_tsx_lines": f"{lib_lines('tsx/'):,}", "json_mb": f"{total_json / 1e6:.1f} MB", "rr_mb": f"{total_rr / 1e6:.1f} MB",
             "atom_rules": f"{atom_rules:,}", "ungrounded": f"{ungrounded:,}", "lib_native": str(n_native), "lib_datalog": str(n_datalog)}

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
