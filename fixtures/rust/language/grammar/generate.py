#!/usr/bin/env python3
"""T1 grammar measurement for a candidate Rust pack (offline, run by a maintainer; never in CI).

Measures tree-sitter-rust 0.24.2 on tree-sitter 0.27.0 (the grammar ast-grep-language 0.45.3 ships, which the
native engine links) against:
- rustc 1.99.0's own parser, the acceptance oracle: `RUSTC_BOOTSTRAP=1 rustc +1.99.0 -Zparse-crate-root-only`
  parses one file exactly as rustc does before expansion (out-of-line `mod x;` is not loaded, so every file is
  judged on its own);
- rust-analyzer's parser (ra_ap_syntax 0.0.357, the parser of rust-analyzer 2026-09), the structural reference:
  a lossless tree with exact spans, compared with tree-sitter's per construct;
- syn 2.0.117 (`full`), the parser proc macros use, as a second acceptance opinion and to read macro bodies.

Pinned corpora (gitignored checkouts under <checkout>/.rerail/corpora/rust/, HEAD verified):
- ripgrep  https://github.com/BurntSushi/ripgrep  3fce3b5bb0236da2df6d99672afb8a719642eca7
- tokio    https://github.com/tokio-rs/tokio      b2636752450484955e7ad334bac678424d51bc4a
- rerail-native: Rerail's native/ (src, build.rs) in the core checkout above this repository (its fixtures/
  submodule), at the commit HEAD names (recorded)
- clippy-ui (stress set, not a real corpus): rust-lang/rust-clippy tests/ui at tag rust-1.99.0,
  49e2f89a2605a9a340e04955fdbefa9803e6027a (<checkout>/.rerail/tools/rust/rust-clippy)

Usage: python3 generate.py [--date YYYY-MM-DD] [--corpus NAME ...]
Writes <checkout>/.rerail/reports/rust-t1-grammar-<date>.json and per-corpus raw data under
<checkout>/.rerail/tools/rust/t1/.

Classes in the report: every file tree-sitter gives an ERROR/MISSING node is `rejected-by-rustc` or
`grammar-gap:<nightly|stable|post-parse-error>:<construct>` (`gap_construct`). Every line where the two trees
disagree on a file both read without error is `misparse:<construct>` (tree-sitter builds a different tree with
no error node) or `convention:<categories>` (a node-boundary convention the harness does not normalize)
(`mismatch_class`).
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import statistics
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[3]


def checkout_root() -> pathlib.Path:
    """The nearest ancestor holding .rerail/corpora/rust (sub-worktrees live under .rerail/wt/)."""
    for parent in [REPO, *REPO.parents]:
        if (parent / ".rerail" / "corpora" / "rust").is_dir():
            return parent
    raise SystemExit("no .rerail/corpora/rust above " + str(REPO))


CHECKOUT = checkout_root()
TOOLS = CHECKOUT / ".rerail" / "tools" / "rust"
TOOLCHAIN = "1.99.0"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
CRATES = {"tree-sitter": "0.27.0", "tree-sitter-rust": "0.24.2", "ra_ap_syntax": "0.0.357", "syn": "2.0.117"}

CORPORA = {
    "ripgrep": {"repository": "https://github.com/BurntSushi/ripgrep",
                "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7",
                "root": CHECKOUT / ".rerail" / "corpora" / "rust" / "ripgrep", "paths": ["."], "kind": "corpus"},
    "tokio": {"repository": "https://github.com/tokio-rs/tokio",
              "commit": "b2636752450484955e7ad334bac678424d51bc4a",
              "root": CHECKOUT / ".rerail" / "corpora" / "rust" / "tokio", "paths": ["."], "kind": "corpus"},
    "rerail-native": {"repository": "https://github.com/neralabs/rerail (native)", "commit": None,
                     "root": REPO, "paths": ["native/src", "native/build.rs"], "kind": "corpus"},
    "clippy-ui": {"repository": "https://github.com/rust-lang/rust-clippy (tests/ui)", "commit": CLIPPY_COMMIT,
                  "root": TOOLS / "rust-clippy", "paths": ["tests/ui"], "kind": "stress"},
}


def run(cmd: list[str], **kw) -> str:
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


def head(root: pathlib.Path) -> str:
    return run(["git", "-C", str(root), "rev-parse", "HEAD"]).strip()


def rust_files(root: pathlib.Path, paths: list[str]) -> list[str]:
    files = []
    for path in paths:
        target = root / path
        found = [target] if target.is_file() else sorted(target.rglob("*.rs"))
        for file in found:
            rel = file.relative_to(root).as_posix()
            if "/target/" in f"/{rel}" or "/node_modules/" in f"/{rel}":
                continue
            files.append(rel)
    return sorted(set(files))


def editions(root: pathlib.Path, name: str, files: list[str]) -> dict[str, str]:
    """Each file's edition: the edition of the deepest Cargo package containing it (cargo metadata), or, in the
    clippy UI suite, its `//@edition:` directive (default 2024, as tests/compile-test.rs sets)."""
    if name == "clippy-ui":
        result = {}
        for rel in files:
            head_text = (root / rel).read_text(errors="replace")[:4000]
            match = re.search(r"//@\s*edition\s*:\s*(\d{4})", head_text)
            result[rel] = match.group(1) if match else "2024"
        return result
    manifests = {}
    for manifest in sorted(root.rglob("Cargo.toml")):
        if "/target/" in manifest.as_posix():
            continue
        text = manifest.read_text()
        if "[package]" not in text:
            continue
        match = re.search(r'^\s*edition\s*=\s*"(\d{4})"', text, re.M)
        if match:
            edition = match.group(1)
        elif re.search(r"^\s*edition\.workspace\s*=\s*true|^\s*edition\s*=\s*\{\s*workspace", text, re.M):
            edition = workspace_edition(manifest.parent, root)
        else:
            edition = "2015"
        manifests[manifest.parent.relative_to(root).as_posix()] = edition
    result = {}
    for rel in files:
        best, edition = -1, "2015"
        for directory, value in manifests.items():
            prefix = "" if directory == "." else directory + "/"
            if rel.startswith(prefix) and len(prefix) > best:
                best, edition = len(prefix), value
        result[rel] = edition
    return result


def workspace_edition(start: pathlib.Path, root: pathlib.Path) -> str:
    for directory in [start, *start.parents]:
        manifest = directory / "Cargo.toml"
        if manifest.is_file() and "[workspace" in manifest.read_text():
            match = re.search(r'\[workspace\.package\][^\[]*?edition\s*=\s*"(\d{4})"', manifest.read_text(), re.S)
            if match:
                return match.group(1)
        if directory == root:
            break
    return "2015"


def rustc_verdict(root: pathlib.Path, rel: str, edition: str) -> dict:
    out_dir = TOOLS / "t1" / "rustc-out"
    proc = subprocess.run(
        ["rustc", f"+{TOOLCHAIN}", "-Zparse-crate-root-only", "--edition", edition, "--error-format=json",
         "--out-dir", str(out_dir), str(root / rel)],
        capture_output=True, text=True, env={**os.environ, "RUSTC_BOOTSTRAP": "1"})
    errors = []
    for line in proc.stderr.splitlines():
        try:
            diag = json.loads(line)
        except ValueError:
            continue
        if diag.get("level") == "error" and diag.get("spans"):
            span = next((s for s in diag["spans"] if s.get("is_primary")), diag["spans"][0])
            errors.append({"message": diag["message"], "line": span["line_start"], "column": span["column_start"]})
        elif diag.get("level") == "error" and not diag["message"].startswith("aborting"):
            errors.append({"message": diag["message"], "line": 0, "column": 0})
    return {"accepted": proc.returncode == 0, "first_error": errors[0] if errors else None}


def harness() -> pathlib.Path:
    target = TOOLS / "target" / "grammar"
    subprocess.run(["cargo", f"+{TOOLCHAIN}", "build", "--release", "-q"], cwd=HERE, check=True,
                   env={**os.environ, "CARGO_TARGET_DIR": str(target)})
    return target / "release" / "rerail-rust-grammar-t1"


def classify(record: dict, verdict: dict) -> str:
    """Syntax-error file classes (rerail-language-conformance.md, T1): rejected by rustc too, or a grammar gap."""
    if not verdict["accepted"]:
        return "rejected-by-rustc"
    return "grammar-gap:" + gap_construct(record)


def gap_construct(record: dict) -> str:
    """Names the construct tree-sitter-rust fails on, from rust-analyzer's tree around the first error and the
    source there. Refined by hand from the samples the report keeps."""
    error = record["ts_error"]
    context = error["ra_context"]
    snippet = error["snippet"]
    ctx = " ".join(context[:6])
    if error["in_token_tree"]:
        return "token-tree"
    for construct, pattern in GAP_PATTERNS:
        if re.search(pattern, snippet):
            return construct
    if "LET_EXPR" in ctx or re.search(r"\b(if|while)\s+let\b.*&&|&&\s*let\b", snippet):
        return "stable:let-chain"
    if "RAW" in ctx or re.search(r"\b(?:raw\s+const|raw\s+mut)\b|&raw\b", snippet):
        return "stable:raw-borrow"
    if re.search(r"\buse\s*<", snippet):
        return "stable:precise-capturing-use"
    if re.search(r"\bunsafe\s+extern\b|\bsafe\s+fn\b", snippet):
        return "stable:unsafe-extern"
    if re.search(r"\bgen\s*(?:move\s*)?\{|\basync\s+gen\b", snippet):
        return "nightly:gen-block"
    return "other:" + (context[0] if context else "unknown")


# Constructs rustc's parser accepts that tree-sitter-rust 0.24.2 does not, named from the clippy-ui stress set
# (the real corpora have none). `nightly:` constructs are feature-gated after parsing: stable code never has them.
GAP_PATTERNS = [
    ("nightly:f16-f128-literal", r"\d[\d_]*(?:\.\d+)?f(?:16|128)\b"),
    ("nightly:const-trait-impl", r"\bconst\s+(?:trait|impl)\b"),
    ("nightly:box-pattern", r"\bbox\s*[\w(]"),
    ("nightly:gen-fn", r"\bgen\s+fn\b"),
    ("nightly:yeet", r"\bdo\s+yeet\b"),
    ("nightly:decl-macro", r"\bmacro\s+\w+\s*[({]"),
    ("nightly:unsafe-field", r"^\s*(?:pub\s+)?unsafe\s+\w+\s*:"),
    ("nightly:default-field-value", r"^\s*(?:pub\s+)?\w+\s*:\s*[\w<>]+\s*=\s*[^=]+,\s*$"),
    ("nightly:postfix-match", r"\.match\s*\{"),
    ("nightly:type-const", r"\btype\s+const\b"),
    ("nightly:closure-binder", r"\bfor\s*<[^>]*>\s*\|"),
    ("nightly:async-use-closure", r"\basync\s+use\b"),
    ("nightly:trait-alias", r"^\s*trait\s+\w+(?:<[^>]*>)?\s*="),
    ("nightly:coroutine-closure-attr", r"#\[coroutine\]"),
    ("stable:numeric-field-in-struct-pattern", r"\{\s*\d+\s*:"),
    ("stable:destructuring-assignment-struct", r"\}\s*=\s*[\w:<>]+\s*[({]"),
    ("stable:unicode-whitespace", "[\u200e\u200f\u2028\u2029\u0085]"),
    ("stable:attribute-on-expression", r"^\s*#\[[^\]]*\]\s*$"),
    ("stable:where-predicate-non-path-type", r"^\s*(?:\(\)|[^:]*\]\s*):\s"),
    ("stable:dyn-lifetime-first-bound", r"\bdyn\s+'"),
    # rustc's parser accepts `1x`; name resolution of the suffix rejects it later.
    ("post-parse-error:literal-unknown-suffix", r"^\s*\d+[a-zA-Z_]\w*\s*;"),
]


def mismatch_class(ms: list) -> str:
    """Classes the structural disagreements of one line (both sides' unmatched spans). `misparse:*` is a tree
    tree-sitter-rust builds without an ERROR node that means something rustc does not (a silent misparse);
    `convention:*` is a span or node-boundary convention the harness does not normalize."""
    texts = [re.sub(r"\s+", "", m["text"]) for m in ms]
    cats = {m["category"] for m in ms}
    ts = [re.sub(r"\s+", "", m["text"]) for m in ms if m["side"] == "tree-sitter"]
    if any(re.search(r"\.\.=?\*", t) for t in texts) and cats & {"expr:binary", "expr:range"}:
        return "misparse:range-end-deref"  # `a..*b` read as `(a..) * b`
    if any(t.startswith("..=") for t in texts) and cats & {"expr:binary", "expr:range"}:
        return "misparse:range-inclusive-no-start"  # `..=b` read as `.. = b`
    if any(re.match(r"break('\w+)?[(&*\-!]", t) for t in ts) or (
            "expr:break" in cats and any(t.startswith("break") for t in texts)):
        return "misparse:break-value"  # `break (x)` read as a call of `break`, `break &x` as a bit-and
    if any(re.match(r"[\w:]+!\{", t) for t in ts) and cats & {"expr:call", "expr:await", "expr:method_call"}:
        return "misparse:brace-macro-statement-call"  # `m! { … } (x)` read as a call of the macro
    if "expr:closure" in cats and any(t.endswith("|_") for t in ts):
        return "misparse:closure-underscore-assign"  # `|x| _ = e` read as `(|x| _) = e`
    return "convention:" + ",".join(sorted(cats))


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def measure(name: str, spec: dict, binary: pathlib.Path) -> dict:
    root = spec["root"]
    commit = head(root)
    if spec["commit"] is not None and commit != spec["commit"]:
        raise SystemExit(f"{root} is at {commit}, the generator pins {spec['commit']}")
    files = rust_files(root, spec["paths"])
    edition_of = editions(root, name, files)
    manifest = hashlib.sha256()
    for rel in files:
        manifest.update(f"{rel}\0{sha256(root / rel)}\n".encode())
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 8) as pool:
        verdicts = dict(zip(files, pool.map(lambda rel: rustc_verdict(root, rel, edition_of[rel]), files)))
    inputs = [{"path": str(root / rel), "rel": rel, "edition": edition_of[rel], "accepted": verdicts[rel]["accepted"]}
              for rel in files]
    raw = json.loads(subprocess.run([str(binary)], input=json.dumps(inputs), capture_output=True, text=True,
                                    check=True).stdout)
    (TOOLS / "t1").mkdir(parents=True, exist_ok=True)
    (TOOLS / "t1" / f"{name}.json").write_text(json.dumps({"verdicts": verdicts, **raw}))
    return summarize(name, spec, commit, files, verdicts, raw, manifest.hexdigest())


def rate(n: int, d: int) -> float | None:
    return round(n / d, 6) if d else None


def summarize(name, spec, commit, files, verdicts, raw, manifest_sha) -> dict:
    records = raw["files"]
    accepted = [r for r in records if verdicts[r["rel"]]["accepted"]]
    rejected = [r for r in records if not verdicts[r["rel"]]["accepted"]]
    total_bytes = sum(r["bytes"] for r in records)
    ts_error_files = [r for r in records if r["ts_error"]]
    ts_on_accepted = [r for r in accepted if r["ts_error"]]
    ra_on_accepted = [r for r in accepted if r["ra_errors"]]
    syn_on_accepted = [r for r in accepted if r["syn_error"]]

    def error_rows(rows, key):
        out = []
        for r in rows:
            e = r[key]
            if key == "ra_first_error":
                out.append({"path": r["rel"], "line": e["at"]["line"], "column": e["at"]["column"],
                            "message": e["message"]})
            else:
                out.append({"path": r["rel"], "line": e["line"], "column": e["column"], "message": e["message"]})
        return out

    classified = []
    for r in ts_error_files:
        e = r["ts_error"]
        classified.append({
            "path": r["rel"], "line": e["at"]["line"], "column": e["at"]["column"], "kind": e["kind"],
            "class": classify(r, verdicts[r["rel"]]), "inTokenTree": e["in_token_tree"],
            "errorNodes": e["error_nodes"], "missingNodes": e["missing_nodes"],
            "raContext": e["ra_context"][:4], "snippet": e["snippet"],
            "rustc": verdicts[r["rel"]]["first_error"]})
    classes = {}
    for row in classified:
        classes[row["class"]] = classes.get(row["class"], 0) + 1

    cats = raw["categories"]
    ts_total = sum(c["ts"] for c in cats.values())
    ra_total = sum(c["ra"] for c in cats.values())
    matched_total = sum(c["matched"] for c in cats.values())
    categories = {
        k: {**v, "agreement": rate(v["matched"], max(v["ts"], v["ra"]))} for k, v in sorted(cats.items())}
    samples = {}
    for m in raw["mismatches"]:
        bucket = samples.setdefault(m["category"], [])
        if len(bucket) < 12:
            bucket.append({k: m[k] for k in ("side", "rel", "line", "column", "text")})
    sites = {}
    for m in raw["mismatches"]:
        sites.setdefault((m["rel"], m["line"]), []).append(m)
    site_classes = {}
    for (rel, line), ms in sorted(sites.items()):
        cls = mismatch_class(ms)
        bucket = site_classes.setdefault(cls, {"sites": 0, "samples": []})
        bucket["sites"] += 1
        if len(bucket["samples"]) < 8:
            bucket["samples"].append({"path": rel, "line": line,
                                      "text": min((m["text"] for m in ms), key=len)[:120]})

    def throughput(key):
        ns = sum(r[key] for r in records)
        per_file = sorted(r[key] / 1e6 for r in records)
        return {"totalMs": round(ns / 1e6, 2), "mibPerSecond": round(total_bytes / (1 << 20) / (ns / 1e9), 2),
                "fileP50Ms": round(statistics.median(per_file), 4),
                "fileP95Ms": round(per_file[int(len(per_file) * 0.95) - 1], 4) if per_file else None}

    macros = raw["macros"]
    by_name = sorted(macros["by_name"].items(), key=lambda kv: -kv[1][1])[:20]
    largest = sorted(records, key=lambda r: -r["bytes"])[:5]
    return {
        "name": name, "kind": spec["kind"], "repository": spec["repository"], "commit": commit,
        "paths": spec["paths"],
        "coverage": {
            "enumeratedFiles": len(files), "sourceBytes": total_bytes, "lines": sum(r["lines"] for r in records),
            "treeSitterNodes": sum(r["ts_nodes"] for r in records), "manifestSha256": manifest_sha,
            "editions": {e: sum(1 for r in records if r["edition"] == e) for e in sorted({r["edition"] for r in records})},
            "largest": [{"path": r["rel"], "bytes": r["bytes"]} for r in largest]},
        "rustc": {"accepted": len(accepted), "rejected": len(rejected),
                  "rejectedFiles": [{"path": r["rel"], **(verdicts[r["rel"]]["first_error"] or {})}
                                    for r in rejected][:200]},
        "acceptedFiles": {
            "treeSitterErrorFiles": len(ts_on_accepted), "treeSitterErrorRate": rate(len(ts_on_accepted), len(accepted)),
            "treeSitterErrorBytesShare": rate(sum(r["bytes"] for r in ts_on_accepted), sum(r["bytes"] for r in accepted)),
            "rustAnalyzerErrorFiles": len(ra_on_accepted), "rustAnalyzerErrorRate": rate(len(ra_on_accepted), len(accepted)),
            "synErrorFiles": len(syn_on_accepted), "synErrorRate": rate(len(syn_on_accepted), len(accepted)),
            "rustAnalyzerErrors": error_rows(ra_on_accepted, "ra_first_error")[:100],
            "synErrors": error_rows(syn_on_accepted, "syn_error")[:100]},
        "rejectedFilesFlagged": {
            "treeSitter": sum(1 for r in rejected if r["ts_error"]),
            "rustAnalyzer": sum(1 for r in rejected if r["ra_errors"]),
            "syn": sum(1 for r in rejected if r["syn_error"])},
        "filesWithSyntaxErrors": {"count": len(ts_error_files), "classes": dict(sorted(classes.items())),
                                  "files": classified},
        "structure": {
            "comparableFiles": raw["comparable_files"], "treeSitterNodes": ts_total, "rustAnalyzerNodes": ra_total,
            "matched": matched_total, "agreement": rate(matched_total, max(ts_total, ra_total)),
            "categories": categories, "mismatchSamples": samples,
            "mismatchSites": dict(sorted(site_classes.items()))},
        "macroBodies": {
            "invocations": macros["invocations"], "bodyBytes": macros["body_bytes"],
            "bodyBytesShare": rate(macros["body_bytes"], total_bytes),
            "macroRulesBytes": sum(r["macro_rules_bytes"] for r in records),
            "macroRulesBytesShare": rate(sum(r["macro_rules_bytes"] for r in records), total_bytes),
            "classes": macros["classes"], "classBytes": macros["class_bytes"],
            "functionsVisible": sum(r["ts_fns"] for r in records),
            "functionsInsideMacroBodies": sum(r["fns_in_macro_bodies"] for r in records),
            "topByBytes": [{"macro": n, "invocations": v[0], "bodyBytes": v[1], "classes": v[2]} for n, v in by_name]},
        "throughput": {"treeSitter": throughput("ts_ns"), "rustAnalyzer": throughput("ra_ns"), "syn": throughput("syn_ns"),
                       "note": "single thread, release build, Apple M-series, parse only (no tree walk)"},
    }


def maintenance() -> dict:
    """tree-sitter-rust's maintenance status (needs gh and network; recorded with the date it was read)."""
    def gh(path: str) -> object:
        return json.loads(run(["gh", "api", path]))
    try:
        repo = gh("repos/tree-sitter/tree-sitter-rust")
        releases = gh("repos/tree-sitter/tree-sitter-rust/releases?per_page=5")
        commits = gh("repos/tree-sitter/tree-sitter-rust/commits?per_page=1")
        issues = json.loads(run(["gh", "api", "search/issues?q=repo:tree-sitter/tree-sitter-rust+is:issue+is:open"]))
        prs = json.loads(run(["gh", "api", "search/issues?q=repo:tree-sitter/tree-sitter-rust+is:pr+is:open"]))
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        return {"error": str(error)}
    return {
        "repository": repo["html_url"], "readOn": datetime.date.today().isoformat(),
        "stars": repo["stargazers_count"], "openIssues": issues["total_count"], "openPullRequests": prs["total_count"],
        "lastPush": repo["pushed_at"], "lastCommit": commits[0]["commit"]["committer"]["date"],
        "releases": [{"tag": r["tag_name"], "published": r["published_at"]} for r in releases]}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--date", default=datetime.date.today().isoformat())
    parser.add_argument("--corpus", action="append", choices=sorted(CORPORA))
    args = parser.parse_args()
    version = run(["rustc", f"+{TOOLCHAIN}", "-V"]).strip()
    if version != RUSTC_VERSION:
        raise SystemExit(f"rustc is {version!r}, the generator pins {RUSTC_VERSION!r}")
    binary = harness()
    names = args.corpus or list(CORPORA)
    results = []
    for name in names:
        print(f"measuring {name}", file=sys.stderr)
        results.append(measure(name, CORPORA[name], binary))
    report = {
        "format": "rerail.language-conformance/v1", "pack": "rust", "tier": "t1-grammar", "date": args.date,
        "kernel": {"path": "none: no Rust family in the kernel yet; tree-sitter-rust measured standalone",
                   "version": f"tree-sitter {CRATES['tree-sitter']}, tree-sitter-rust {CRATES['tree-sitter-rust']} "
                              "(ast-grep-language 0.45.3)"},
        "corpora": [{"name": r["name"], "repository": r["repository"], "commit": r["commit"]} for r in results],
        "tools": [{"name": "rustc (-Zparse-crate-root-only)", "version": RUSTC_VERSION},
                  {"name": "ra_ap_syntax", "version": CRATES["ra_ap_syntax"]},
                  {"name": "syn", "version": CRATES["syn"]},
                  {"name": "tree-sitter-rust", "version": CRATES["tree-sitter-rust"]},
                  {"name": "tree-sitter", "version": CRATES["tree-sitter"]}],
        "benchmark": "tree-sitter-rust vs rustc acceptance and rust-analyzer structure",
        "method": (
            "Every .rs file of each corpus is parsed by rustc 1.99.0 (-Zparse-crate-root-only, the file's Cargo "
            "edition), tree-sitter-rust 0.24.2, ra_ap_syntax 0.0.357 and syn 2.0.117. Error rates are over the files "
            "rustc accepts. Structure compares, on files rustc accepts and both tree-sitter and rust-analyzer read "
            "without error, the multiset of spans each grammar gives every comparable construct (items, expressions, "
            "patterns, types, signatures, attributes, comments, macro calls and token trees), after normalizing "
            "node-boundary conventions (src/main.rs `ts_category`/`ra_category`): rust-analyzer's item spans start "
            "after their attributes and doc comments; match arms and block labels are compared without the "
            "trailing comma and label; tree-sitter's one-element tuple type/pattern without a comma is a paren "
            "type/pattern; a typed `self: T` is a self parameter; unnamed fn-pointer parameters, attribute "
            "arguments (literals, macro calls, cfg/cfg_attr predicates, `unsafe(...)`), ABI strings and tuple "
            "indices are not compared; let chains are compared at their outermost `&&`. The remaining "
            "disagreements are classed per line in `structure.mismatchSites`: `misparse:*` (tree-sitter builds a "
            "different tree with no error node) or `convention:*`. Macro bodies are the token trees of outermost "
            "macro invocations; syn classifies each as items, expressions, statements, macro_rules metavariable "
            "fragments or a custom DSL."),
        "corpusResults": results,
        "grammarMaintenance": maintenance(),
    }
    out_dir = CHECKOUT / ".rerail" / "reports"
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f"rust-t1-grammar-{args.date}.json"
    out.write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n")
    print(out)


if __name__ == "__main__":
    main()
