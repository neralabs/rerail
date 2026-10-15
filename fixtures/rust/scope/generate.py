#!/usr/bin/env python3
"""Record rust-analyzer SCIP scope occurrences and rustc unused-name diagnostics.

Pins rust-analyzer/rustc/cargo to 1.99.0. Usage: python3 generate.py.
The expected file is tool output, not Rerail output; SCIP UTF-8 positions are
converted to one-based Unicode code-point columns. Symbol display strings are
SCIP descriptors with the package prefix and terminal descriptor suffixes
rendered as qualified paths where possible.
"""
from __future__ import annotations
import json, os, pathlib, re, subprocess

HERE = pathlib.Path(__file__).resolve().parent
CRATE = HERE / "crate"
ROOT = HERE.parents[2]
BIN = pathlib.Path.home() / ".rustup/toolchains/1.99.0-aarch64-apple-darwin/bin"
BUILD = ROOT / ".rerail/tools/rust/scope"
DECODER = ROOT / ".rerail/tools/rust/target/scope-decoder/release/rerail-rust-feasibility"
RA = BIN / "rust-analyzer"
TOOLCHAIN = "1.99.0"
RA_VERSION = "rust-analyzer 1.99.0 (b940084d 2026-09-28)"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def source(path):
    return pathlib.Path(path).read_text(encoding="utf-8")


def pos(text, line, char):
    lines = text.splitlines()
    return lines[line - 1][char:]


def display(symbol):
    # SCIP descriptors are `rust-analyzer cargo package version ...` and use
    # `.` for modules, `#` for types, `/` for fields, `:` for methods, `!` macros.
    parts = symbol.split()
    if len(parts) < 5:
        return symbol
    desc = parts[-1]
    desc = re.sub(r"[#!/:]", "", desc)
    return "::".join(x for x in desc.split(".") if x)


def main():
    if (run([str(RA), "--version"]).stdout + run([str(RA), "--version"]).stderr).strip() != RA_VERSION:
        raise SystemExit("rust-analyzer pin mismatch")
    if run(["rustc", "+" + TOOLCHAIN, "-V"]).stdout.strip() != RUSTC_VERSION:
        raise SystemExit("rustc pin mismatch")
    BUILD.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, RUSTUP_TOOLCHAIN=TOOLCHAIN, CARGO_TARGET_DIR=str(BUILD / "cargo"))
    scip = BUILD / "scope.scip"
    config = BUILD / "ra-config.json"
    config.write_text(json.dumps({"cargo": {"features": "all"}}) + "\n")
    subprocess.run([str(RA), "scip", str(CRATE), "--config-path", str(config), "--output", str(scip)], check=True, env=env)
    if not DECODER.exists():
        subprocess.run(["cargo", "+" + TOOLCHAIN, "build", "--release", "--manifest-path", str(ROOT / "fixtures/rust/language/feasibility/Cargo.toml")], check=True,
                       env=dict(os.environ, CARGO_TARGET_DIR=str(ROOT / ".rerail/tools/rust/target/scope-decoder")))
    decoded = BUILD / "scope.scip.jsonl"
    subprocess.run([str(DECODER), "scip", str(scip), str(decoded)], check=True)
    lines = decoded.read_text().splitlines()
    docs = [json.loads(x) for x in lines[:-1]]
    meta = json.loads(lines[-1])
    symbols = {}
    symbol_kinds = {s: k for doc in docs for s, k in doc.get("syms", [])}
    external_symbols = set(meta.get("external", []))
    occurrences = []
    files = []
    for doc in docs:
        raw = pathlib.Path(doc["path"])
        if not raw.is_absolute():
            raw = CRATE / raw
        path = str(raw.relative_to(CRATE.parent.parent)) if raw.is_absolute() else str(raw)
        path = path.replace("\\", "/")
        files.append(path)
        text = source(raw)
        for start_line, start_char, end_line, end_char, sym, roles in doc["occ"]:
            start_line += 1
            end_line += 1 if end_char else 0
            if isinstance(sym, int):
                sym = meta["symbols"][sym]
            sym = str(sym)
            # rust-analyzer currently emits UTF-8 byte columns; decode prefix safely.
            line_text = text.splitlines()[start_line - 1]
            prefix = line_text.encode("utf-8")[:start_char].decode("utf-8")
            end_prefix = line_text.encode("utf-8")[:end_char].decode("utf-8") if end_line == start_line else text.splitlines()[end_line-1].encode("utf-8")[:end_char].decode("utf-8")
            if end_line == start_line:
                occ_text = line_text[len(prefix):len(end_prefix)]
            else:
                source_lines = text.splitlines()
                occ_text = "\n".join([line_text[len(prefix):], *source_lines[start_line:end_line-1], source_lines[end_line-1][:len(end_prefix)]])
            item = {"path": path, "line": start_line, "column": len(prefix)+1, "end_line": end_line, "end_column": len(end_prefix)+1, "text": occ_text, "symbol": sym,
                    "definition": bool(roles & 1), "write": bool(roles & 4), "read": not bool(roles & 1)}
            occurrences.append(item)
            if sym not in symbols:
                symbols[sym] = {"kind": symbol_kinds.get(sym, 0), "definition": None, "external": sym in external_symbols, "display": display(sym)}
            if item["definition"] and symbols[sym]["definition"] is None:
                symbols[sym]["definition"] = {"path": path, "line": start_line, "column": len(prefix)+1}
    files = sorted(set(files))
    occurrences.sort(key=lambda x: (x["path"], x["line"], x["column"], x["symbol"]))
    # rustc diagnostics are JSON lines; retain only requested unused lints.
    proc = subprocess.run(["cargo", "+" + TOOLCHAIN, "check", "--manifest-path", str(CRATE / "Cargo.toml"), "--message-format=json"], capture_output=True, text=True, env=env)
    lints = []
    wanted = {"unused_variables", "unused_imports", "dead_code", "unused_mut", "unused_assignments", "unused_labels"}
    for line in proc.stdout.splitlines():
        try: msg = json.loads(line)
        except json.JSONDecodeError: continue
        if msg.get("reason") != "compiler-message": continue
        m = msg["message"]; code = (m.get("code") or {}).get("code")
        if code not in wanted or not m.get("spans"): continue
        span = next((s for s in m["spans"] if s.get("is_primary")), m["spans"][0])
        p = pathlib.Path(span["file_name"])
        p = str(p.relative_to(CRATE.parent.parent)).replace("\\", "/") if p.is_absolute() else str(CRATE.relative_to(CRATE.parent.parent) / p)
        lints.append({"lint": code, "path": p, "line": span["line_start"], "column": span["column_start"], "text": span.get("text", [{"text":""}])[0]["text"], "message": m["message"]})
    # Conservative silent scan: identifiers absent from SCIP occurrences, skipping comments/strings.
    seen = {(o["path"], o["line"], o["column"]) for o in occurrences}
    silent = []
    ident = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
    for p in sorted(CRATE.rglob("*.rs")):
        rel = str(p.relative_to(CRATE.parent.parent)).replace("\\", "/"); raw = p.read_text()
        for n, line in enumerate(raw.splitlines(), 1):
            clean = re.sub(r'//.*|"(?:\\.|[^"\\])*"|\'[^\'\\]*(?:\\.[^\'\\]*)*\'', "", line)
            for m in ident.finditer(clean):
                col = m.start()+1
                if (rel,n,col) not in seen and m.group() not in {"Rerail", "authored", "fixture", "Apache", "License"}:
                    silent.append({"path": rel, "line": n, "column": col, "text": m.group()})
    record = {"tools": {"rust-analyzer": RA_VERSION, "rustc": RUSTC_VERSION}, "files": files, "occurrences": occurrences, "symbols": dict(sorted(symbols.items())), "lints": sorted(lints, key=lambda x:(x["path"],x["line"],x["column"])), "silent": silent}
    (HERE / "scope.expected.json").write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n")
    print(json.dumps({"occurrences":len(occurrences), "definitions":sum(o["definition"] for o in occurrences), "references":sum(o["read"] for o in occurrences), "external_symbols":sum(v["external"] for v in symbols.values()), "lints":len(lints), "silent":len(silent)}))

if __name__ == "__main__": main()
