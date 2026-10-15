#!/usr/bin/env python3
"""Record Rust call resolution from pinned rust-analyzer and the syn scanner.

Uses rust-analyzer 1.99.0 and rustc/cargo 1.99.0. The crate is copied under
.rerail/tools so rust-analyzer sees a standalone workspace. Calls come only
from the feasibility scanner (never from text heuristics); SCIP occurrences
supply resolution. Positions in the recording are one-based Unicode
code-point columns. Build products and raw indexes are intentionally kept out
of the fixture tree.
"""
from __future__ import annotations
import json, os, pathlib, re, shutil, subprocess

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
TOOLCHAIN = "1.99.0"
BIN = pathlib.Path.home()/".rustup"/"toolchains"/"1.99.0-aarch64-apple-darwin"/"bin"
TOOLS = REPO/".rerail/tools/rust/calls"
SCANNER_TARGET = REPO/".rerail/tools/rust/calls-scanner"
WORK = TOOLS/"work/crate"
OUT = TOOLS/"out"


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw)


def pos(text: str, off: int):
    line = text.count("\n", 0, off) + 1
    start = text.rfind("\n", 0, off) + 1
    return line, len(text[start:off]) + 1


def source_offset(text: str, line: int, char: int, encoding: int) -> int:
    lines = text.splitlines(keepends=True)
    s = lines[line] if line < len(lines) else ""
    prefix = s[:char]
    base = len("".join(lines[:line]).encode("utf-8"))
    if encoding == 1:  # SCIP UTF-8 byte columns
        return base + len(prefix.encode("utf-8"))
    if encoding == 2:  # UTF-16 code units
        return base + len(prefix.encode("utf-16-le")) // 2
    return base + char


def token_at(text: str, off: int) -> str:
    # Only extracts the already scanner-selected token, not call discovery.
    a = off
    while a > 0 and (text[a-1].isalnum() or text[a-1] == "_"): a -= 1
    b = off
    while b < len(text) and (text[b].isalnum() or text[b] == "_"): b += 1
    return text[a:b]


def main():
    found = run([str(BIN/"rust-analyzer"), "--version"]).stdout.strip()
    if found != "rust-analyzer 1.99.0 (b940084d 2026-09-28)": raise SystemExit(found)
    run(["cargo", "+"+TOOLCHAIN, "check", "--manifest-path", str(HERE/"crate/Cargo.toml")])
    if TOOLS.exists(): shutil.rmtree(TOOLS)
    OUT.mkdir(parents=True)
    WORK.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(HERE/"crate", WORK)
    (TOOLS/"config.json").write_text('{"cargo":{"features":"all"}}\n')
    env = dict(os.environ, RUSTUP_TOOLCHAIN=TOOLCHAIN,
               CARGO_TARGET_DIR=str(REPO/".rerail/tools/rust/calls-target"))
    run([str(BIN/"rust-analyzer"), "scip", str(WORK), "--config-path", str(TOOLS/"config.json"),
         "--output", str(OUT/"index.scip")], env=env)
    scanner = SCANNER_TARGET/"release/rerail-rust-feasibility"
    if not scanner.exists():
        run(["cargo", "+"+TOOLCHAIN, "build", "--release", "--locked", "--manifest-path",
             str(HERE/"../language/feasibility/Cargo.toml")],
            env=dict(os.environ, CARGO_TARGET_DIR=str(SCANNER_TARGET)))
    run([str(scanner), "scip", str(OUT/"index.scip"), str(OUT/"scip.jsonl")])
    run([str(scanner), "scan", str(WORK), str(OUT/"scan.jsonl")])

    scans = {}
    for line in (OUT/"scan.jsonl").read_text().splitlines():
        d=json.loads(line); scans[d["path"]]=d
    docs=[]
    lines=(OUT/"scip.jsonl").read_text().splitlines()
    meta=json.loads(lines[-1])
    for line in lines[:-1]: docs.append(json.loads(line))
    by_path={d["path"]:d for d in docs}
    symbols=meta["symbols"]
    occurrences={}; definitions={}; ext_symbols=set()
    for d in docs:
        path=d["path"]; text=(WORK/path).read_text(encoding="utf-8"); enc=d.get("encoding",1)
        for o in d.get("occ",[]):
            off=source_offset(text,o[0],o[1],enc); occurrences[(path,off)]=o[4]
            if o[5] & 1: definitions.setdefault(o[4], (path, o[0]+1, o[1]+1))
    crate_prefix="rust-analyzer cargo rerail_calls_fixture 0.1.0 "
    def_rows=[]
    for sym,(path,line,col) in sorted(definitions.items(), key=lambda x:(x[1][0],x[1][1],x[1][2],x[0])):
        display=symbols[sym] if isinstance(sym,int) else ""
        if display.endswith("crate/"): continue
        tail=display[len(crate_prefix):] if display.startswith(crate_prefix) else display
        name=tail.rsplit("/",1)[-1].rstrip(".")
        if name.endswith("()"): name=name[:-2]
        kind="constructor" if "#" in name else "function"
        def_rows.append({"symbol":display,"path":path,"line":line,"column":col,"name":name,"kind":kind})
    external_names={symbols[i] for i in range(len(symbols)) if i not in definitions}
    calls=[]
    for path,sc in sorted(scans.items()):
        if path not in by_path: continue
        text=(WORK/path).read_text(encoding="utf-8")
        for off,kind in sc.get("calls",[]):
            bare=kind
            if bare.startswith("m:"): bare=bare[2:]
            line,col=pos(text,off); sym=occurrences.get((path,off))
            target=None
            if sym is not None and sym in definitions:
                dp,dl,dc=definitions[sym]; target={"path":dp,"line":dl,"column":dc}
            elif sym is not None:
                target={"external":symbols[sym]}
            row={"path":path,"line":line,"column":col,"text":token_at(text,off),"kind":kind,"symbol":symbols[sym] if sym is not None else None,"target":target}
            if any(a <= off <= b for a,b in sc.get("cfg",[])): row["cfg"]="test"
            calls.append(row)
    record={"tools":{"rust-analyzer":found,"rustc":run(["rustc","+"+TOOLCHAIN,"-V"]).stdout.strip(),"scanner":"syn 2.0.119 / proc-macro2 1.0.107"},
            "files":sorted(scans),"definitions":def_rows,"calls":calls}
    (HERE/"calls.expected.json").write_text(json.dumps(record,indent=1,ensure_ascii=False)+"\n")
    from collections import Counter
    c=Counter(x["kind"] for x in calls); resolved=sum(isinstance(x["target"],dict) and "path" in x["target"] for x in calls); ext=sum(isinstance(x["target"],dict) and "external" in x["target"] for x in calls)
    print(json.dumps({"calls":len(calls),"by_kind":dict(sorted(c.items())),"crate_definitions":resolved,"external":ext,"unresolved":len(calls)-resolved-ext},sort_keys=True))

if __name__ == "__main__": main()
