#!/usr/bin/env python3
"""Feasibility census for a Rust pack's semantic capabilities (offline, run by a maintainer; never in CI).

Writes three small summaries beside this file, keyed by corpus:
- names.json: every reference occurrence rust-analyzer's SCIP index reports, classified by target (local binding,
  item in the same file / another file of the same package / another workspace package / an external crate) and
  by syntactic position (from syn), with the decision a syntax-only resolver would need (`syntax`, `syntax_extern`,
  `types`, `expansion`), both with no macro handling (`strict`) and with re-parsable macro token trees read as
  their contents (`transparent`); plus call sites, operator dispatch sites, macro hygiene counters and the
  identifiers rust-analyzer leaves without an occurrence (`silent`).
- values.json: every constant-expression site (const/static/associated const items, enum discriminants, array
  and repeat lengths, const generic arguments) classified by initializer shape, and the share a literal +
  arithmetic evaluator decides, alone and when it follows references to other constants by name.
- suppression.json: every lint-level attribute (`allow`/`expect`/`warn`/`deny`/`forbid`, also under `cfg_attr`,
  inside macro token trees and `macro_rules!` bodies), by level, placement, lint namespace and group.

Pins (refuses anything else):
- toolchain 1.99.0: rustc 1.99.0 (b940084d7 2026-09-28), rust-analyzer 1.99.0 (b940084d 2026-09-28),
  clippy 0.1.99 (b940084d7e 2026-09-28) (lint and group names, `-W help`)
- rust-clippy source rust-1.99.0 = 49e2f89a2605a9a340e04955fdbefa9803e6027a (renamed/deprecated lint names)
- the scanner beside this file (main.rs; syn 2.0.119, proc-macro2 1.0.107, serde_json 1.0.151, Cargo.lock)
- corpora (gitignored shallow clones under <checkout>/.rerail/corpora/rust/, HEAD verified):
  ripgrep https://github.com/BurntSushi/ripgrep 3fce3b5bb0236da2df6d99672afb8a719642eca7
  tokio   https://github.com/tokio-rs/tokio     b2636752450484955e7ad334bac678424d51bc4a

Pipeline per corpus: copy the checkout (without .git) to <checkout>/.rerail/tools/rust/feasibility/work/<corpus>
(rust-analyzer runs cargo there), `rust-analyzer scip` with all cargo features enabled and
CARGO_TARGET_DIR=<checkout>/.rerail/tools/rust/target/feasibility-<corpus>, decode the index and scan the same
files with the syn scanner; raw dumps stay in <checkout>/.rerail/tools/rust/feasibility/out/.

Usage: python3 generate.py [--reuse]   (--reuse skips copy, index and scan when the raw dumps exist)

Caveats, recorded in the summaries too: rust-analyzer is the oracle, so code it does not index (inactive
`#[cfg]`, other targets' platform code) is `silent`, not counted as references; a SCIP occurrence is joined to
syn's identifier by byte offset; `same_crate` means the same cargo package (bins, tests and examples of a package
share its symbol prefix); `const_ref` and constant following go by SCREAMING_CASE / one-letter names, not by
resolution.
"""

from __future__ import annotations

import argparse
import bisect
import collections
import json
import os
import pathlib
import re
import shutil
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]


def checkout_root() -> pathlib.Path:
    """The nearest ancestor holding .rerail/corpora/rust (worktrees live under .rerail/wt/)."""
    for parent in [REPO, *REPO.parents]:
        if (parent / ".rerail" / "corpora" / "rust").is_dir():
            return parent
    raise SystemExit("no .rerail/corpora/rust above " + str(REPO))


CHECKOUT = checkout_root()
TOOLS = CHECKOUT / ".rerail" / "tools" / "rust"
RAW = TOOLS / "feasibility"
TOOLCHAIN = "1.99.0"
BIN = pathlib.Path.home() / ".rustup" / "toolchains" / "1.99.0-aarch64-apple-darwin" / "bin"
VERSIONS = {
    "rustc": "rustc 1.99.0 (b940084d7 2026-09-28)",
    "rust-analyzer": "rust-analyzer 1.99.0 (b940084d 2026-09-28)",
    "clippy-driver": "clippy 0.1.99 (b940084d7e 2026-09-28)",
}
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
SCANNER = {"syn": "2.0.119", "proc-macro2": "1.0.107", "serde_json": "1.0.151"}
CORPORA = {
    "ripgrep": {"repository": "https://github.com/BurntSushi/ripgrep",
                "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7"},
    "tokio": {"repository": "https://github.com/tokio-rs/tokio",
              "commit": "b2636752450484955e7ad334bac678424d51bc4a"},
}
RA_CONFIG = {"cargo": {"features": "all"}}

PRIMITIVES = {"bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
              "isize", "f16", "f32", "f64", "f128"}
KEYWORDS = {"self", "Self", "super", "crate"}
# Standard numeric associated constants a literal evaluator can table (`u32::MAX`, `f64::EPSILON`, ...).
STD_CONSTS = {"MAX", "MIN", "BITS", "EPSILON", "MIN_POSITIVE", "INFINITY", "NEG_INFINITY", "NAN", "RADIX",
              "MANTISSA_DIGITS", "DIGITS", "MAX_EXP", "MIN_EXP", "MAX_10_EXP", "MIN_10_EXP", "REPLACEMENT_CHARACTER",
              "UNICODE_VERSION"}
# SCIP SymbolInformation.Kind values used below.
KIND_TRAIT = 53
KIND_CTOR = {12, 49, 59}  # EnumMember, Struct, Union

IDENT = re.compile(rb"(?:r#)?[A-Za-z_][A-Za-z0-9_]*")


def run(cmd: list[str], **kw) -> str:
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


def check_pins() -> None:
    found = {
        "rustc": run(["rustc", "+" + TOOLCHAIN, "-V"]).strip(),
        "rust-analyzer": run([str(BIN / "rust-analyzer"), "--version"]).strip(),
        "clippy-driver": run([str(BIN / "clippy-driver"), "--version"]).strip(),
    }
    for tool, want in VERSIONS.items():
        if found[tool] != want:
            raise SystemExit(f"{tool}: want {want!r}, found {found[tool]!r}")
    clippy = TOOLS / "rust-clippy"
    if run(["git", "-C", str(clippy), "rev-parse", "HEAD"]).strip() != CLIPPY_COMMIT:
        raise SystemExit(f"{clippy} is not at {CLIPPY_COMMIT}")
    for name, pin in CORPORA.items():
        root = CHECKOUT / ".rerail" / "corpora" / "rust" / name
        if run(["git", "-C", str(root), "rev-parse", "HEAD"]).strip() != pin["commit"]:
            raise SystemExit(f"{root} is not at {pin['commit']}")
        if run(["git", "-C", str(root), "status", "--porcelain"]).strip():
            raise SystemExit(f"{root} has local changes")


# ---- raw data ----

def scanner() -> pathlib.Path:
    env = dict(os.environ, CARGO_TARGET_DIR=str(RAW / "target"))
    subprocess.run(["cargo", "+" + TOOLCHAIN, "build", "--release", "--locked", "-q", "--manifest-path",
                    str(HERE / "Cargo.toml")], check=True, env=env)
    return RAW / "target" / "release" / "rerail-rust-feasibility"


def produce(name: str, tool: pathlib.Path) -> None:
    source = CHECKOUT / ".rerail" / "corpora" / "rust" / name
    work = RAW / "work" / name
    out = RAW / "out"
    out.mkdir(parents=True, exist_ok=True)
    if work.exists():
        shutil.rmtree(work)
    shutil.copytree(source, work, ignore=shutil.ignore_patterns(".git"))
    config = RAW / "ra-config.json"
    config.write_text(json.dumps(RA_CONFIG) + "\n")
    env = dict(os.environ, RUSTUP_TOOLCHAIN=TOOLCHAIN,
               CARGO_TARGET_DIR=str(TOOLS / "target" / ("feasibility-" + name)))
    with open(out / (name + ".scip.log"), "w") as log:
        subprocess.run([str(BIN / "rust-analyzer"), "scip", str(work), "--config-path", str(config),
                        "--output", str(out / (name + ".scip"))], check=True, env=env, stdout=log, stderr=log)
    subprocess.run([str(tool), "scip", str(out / (name + ".scip")), str(out / (name + ".scip.jsonl"))], check=True)
    subprocess.run([str(tool), "scan", str(work), str(out / (name + ".scan.jsonl"))], check=True)


def load(name: str):
    out = RAW / "out"
    lines = (out / (name + ".scip.jsonl")).read_text().splitlines()
    docs = [json.loads(line) for line in lines[:-1]]
    meta = json.loads(lines[-1])
    scan = {}
    with open(out / (name + ".scan.jsonl")) as handle:
        for line in handle:
            entry = json.loads(line)
            scan[entry["path"]] = entry
    return docs, meta, scan


# ---- SCIP symbols ----

def descriptors(text: str) -> list[tuple[str, str, int]]:
    """SCIP descriptors as (name, suffix, end offset); suffix one of / # . ! : method typeparam param."""
    out: list[tuple[str, str, int]] = []
    i = 0

    def name_at(i: int, stop: str) -> tuple[str, int]:
        if text[i] == "`":
            j = i + 1
            buf = []
            while j < len(text):
                if text[j] == "`":
                    if j + 1 < len(text) and text[j + 1] == "`":
                        buf.append("`")
                        j += 2
                        continue
                    return "".join(buf), j + 1
                buf.append(text[j])
                j += 1
            return "".join(buf), j
        j = i
        while j < len(text) and text[j] not in stop:
            j += 1
        return text[i:j], j

    while i < len(text):
        c = text[i]
        if c == "[":
            name, i = name_at(i + 1, "]")
            i += 1
            out.append((name, "typeparam", i))
        elif c == "(":
            name, i = name_at(i + 1, ")")
            i += 1
            out.append((name, "param", i))
        else:
            name, i = name_at(i, "/#.!:([")
            if i >= len(text):
                out.append((name, "?", i))
                break
            s = text[i]
            if s == "(":
                close = text.index(")", i)
                i = close + 2  # `().`
                out.append((name, "method", i))
            else:
                i += 1
                out.append((name, s, i))
    return out


class Symbols:
    def __init__(self, meta: dict, docs: list[dict]):
        self.names: list[str] = meta["symbols"]
        self.kind: dict[str, int] = {}
        for doc in docs:
            for sym, kind in doc["syms"]:
                self.kind[self.names[sym]] = kind
        for sym, kind in meta["external"]:
            self.kind.setdefault(self.names[sym], kind)
        self.cache: dict[int, tuple] = {}

    def info(self, sym: int) -> tuple:
        """(package or None for locals, member class, parent symbol, kind) for a symbol id."""
        if sym in self.cache:
            return self.cache[sym]
        text = self.names[sym]
        if text.startswith("local "):
            result = (None, "local", None, None)
        else:
            parts = text.split(" ", 4)
            package, desc_text = parts[2], parts[4] if len(parts) > 4 else ""
            prefix = len(text) - len(desc_text)
            desc = descriptors(desc_text)
            member, parent = "free", None
            if len(desc) >= 2 and desc[-1][1] in ("method", ".", "#"):
                # Walk back over type parameters to the owner of the last descriptor.
                k = len(desc) - 2
                params = 0
                while k >= 0 and desc[k][1] == "typeparam":
                    params += 1
                    k -= 1
                if k >= 0 and desc[k][1] == "#":
                    if desc[k][0] == "impl":
                        member = "inherent" if params <= 1 else "trait_impl"
                    else:
                        parent = text[: prefix + desc[k][2]]
                        member = "trait_item" if self.kind.get(parent) == KIND_TRAIT else "type_member"
            result = (package, member, parent, self.kind.get(text))
        self.cache[sym] = result
        return result


# ---- per-file geometry ----

class Ranges:
    """Sorted, possibly nested byte ranges; `inside` answers containment."""

    def __init__(self, ranges: list[list[int]]):
        self.ranges = sorted((a, b) for a, b in ranges)
        self.starts = [a for a, _ in self.ranges]

    def inside(self, at: int) -> bool:
        k = bisect.bisect_right(self.starts, at)
        return any(a <= at < b for a, b in self.ranges[:k])

    def innermost(self, at: int):
        best = None
        k = bisect.bisect_right(self.starts, at)
        for a, b in self.ranges[:k]:
            if a <= at < b and (best is None or b - a < best[1] - best[0]):
                best = (a, b)
        return best


class File:
    def __init__(self, root: pathlib.Path, scan: dict):
        self.data = (root / scan["path"]).read_bytes()
        self.starts = [0] + [m.end() for m in re.finditer(b"\n", self.data)]
        self.scan = scan
        self.idents = {entry[0]: entry for entry in scan["idents"]}
        self.opaque = Ranges(scan["opaque"])
        self.parsed = Ranges(scan["parsed"])
        self.rules = Ranges(scan["rules"])
        self.strs = Ranges(scan["strs"])
        self.cfg = Ranges(scan["cfg"])
        self.fns = Ranges(scan["fns"])

    def offset(self, line: int, column: int) -> int:
        return self.starts[line] + column

    def ident(self, at: int) -> str:
        m = IDENT.match(self.data, at)
        return m.group(0).decode() if m else ""

    def macro_context(self, at: int) -> str:
        """`opaque` / `parsed` / `rules` when `at` sits in a macro token tree or macro_rules body, else ``."""
        if self.rules.inside(at):
            return "rules"
        if self.opaque.inside(at):
            return "opaque"
        if self.parsed.inside(at):
            return "parsed"
        return ""


def counter_dict(c: collections.Counter) -> dict:
    return {" / ".join(k) if isinstance(k, tuple) else k: v for k, v in sorted(c.items(), key=lambda kv: (-kv[1], str(kv[0])))}


def share(part: int, whole: int) -> float:
    return round(part / whole, 4) if whole else 0.0


# ---- names, scope and calls ----

PATHISH = {"path_single", "path_prefix", "path_last", "path_qself", "use", "macro_name", "attr", "derive",
           "pat_ident", "assoc_binding", "field_init", "field_pat", "field_shorthand", "field_pat_shorthand",
           "mod_decl", "lifetime", "format_str"}
POSITION_GROUP = {
    "path_single": "path", "path_prefix": "path", "path_last": "path", "path_qself": "path", "pat_ident": "path",
    "assoc_binding": "path", "use": "use", "macro_name": "macro_name", "attr": "attribute", "derive": "attribute",
    "method": "method", "field": "field", "tuple_field": "field", "field_init": "struct_field",
    "field_pat": "struct_field", "field_shorthand": "struct_field", "field_pat_shorthand": "struct_field",
    "mod_decl": "path", "lifetime": "lifetime", "format_str": "format_args_inline", "opaque_tt": "opaque_macro",
    "rules": "macro_rules_body",
}


def names(name: str) -> dict:
    docs, meta, scan = load(name)
    root = RAW / "work" / name
    syms = Symbols(meta, docs)
    files = {path: File(root, entry) for path, entry in scan.items()}

    # Definitions: symbol -> [(path, offset)]; document package = its most frequent defined package.
    defs: dict[int, list] = collections.defaultdict(list)
    package_of: dict[str, str] = {}
    occs: dict[str, list] = {}
    for doc in docs:
        f = files[doc["path"]]
        count = collections.Counter()
        rows = []
        for sl, sc, el, ec, sym, roles in doc["occ"]:
            at = f.offset(sl, sc)
            end = f.offset(el, ec)
            rows.append((at, end, sym, roles))
            if roles & 1:
                defs[sym].append((doc["path"], at))
                package = syms.info(sym)[0]
                if package:
                    count[package] += 1
        package_of[doc["path"]] = count.most_common(1)[0][0] if count else ""
        occs[doc["path"]] = rows
    workspace = sorted(set(package_of.values()) - {""})

    def target(path: str, at: int, sym: int) -> str:
        package, _, _, _ = syms.info(sym)
        if package is None:
            sites = [d for d in defs[sym] if d[0] == path]
            if sites:
                f = files[path]
                return "local_same_fn" if f.fns.innermost(at) == f.fns.innermost(sites[0][1]) else "local_outer"
            # A local whose definition token carries another symbol: macro input expanded more than once
            # (`$fun` pasted twice); still a binding written in this file.
            return "local_multi_expansion"
        if package == package_of[path]:
            if any(d[0] == path for d in defs[sym]):
                return "same_file"
            return "same_crate" if defs[sym] else "same_crate_nodef"
        if package in workspace:
            return "workspace" if defs[sym] else "workspace_nodef"
        return "external"

    def defined_in_macro(sym: int) -> str:
        """`opaque`/`parsed`/`rules` when every definition site lies in a macro token tree, else ``."""
        sites = defs.get(sym)
        if not sites:
            return ""
        contexts = {files[p].macro_context(at) for p, at in sites}
        if "" in contexts:
            return ""
        return "opaque" if contexts & {"opaque", "rules"} else "parsed"

    def base_decision(pos: str, tgt: str, sym: int, prev_sym) -> str:
        if pos in ("method", "field", "tuple_field"):
            return "types"
        if tgt.endswith("_nodef"):
            return "expansion"  # an item whose name only exists after expansion (pin_project!, mockall, ...)
        _, member, parent, _ = syms.info(sym)
        if member == "trait_impl" and pos not in ("use",):
            return "types"
        if member == "trait_item" and pos in ("path_last", "path_qself", "pat_ident", "path_single"):
            if prev_sym is None or syms.names[prev_sym] != parent:
                return "types"
        return "syntax_extern" if tgt == "external" else "syntax"

    position = collections.Counter()      # (group, target)
    strict = collections.Counter()        # (group, decision)
    transparent = collections.Counter()
    decision_total = {"strict": collections.Counter(), "transparent": collections.Counter()}
    silent = collections.Counter()
    silent_samples: dict[str, list] = collections.defaultdict(list)
    hygiene = collections.Counter()
    operators = collections.Counter()
    calls = collections.Counter()
    call_decision = {"strict": collections.Counter(), "transparent": collections.Counter()}
    macros = collections.Counter()

    for path in sorted(files):
        f = files[path]
        rows = occs.get(path, [])
        by_offset: dict[int, list] = collections.defaultdict(list)
        for at, end, sym, roles in rows:
            by_offset[at].append((end, sym, roles))
        refs: dict[int, int] = {}
        for at, end, sym, roles in rows:
            if roles & 1:
                continue
            token = f.data[at:end]
            entry = f.idents.get(at)
            in_macro = False
            if entry is not None:
                kind = entry[1]
                if kind.startswith("m:"):
                    in_macro = True
                    kind = kind[2:]
                pos = kind
            elif token.startswith(b"'"):
                pos = "lifetime"
            elif token.isdigit():
                pos = "tuple_field"
            elif IDENT.fullmatch(token):
                ctx = f.macro_context(at)
                if ctx == "rules":
                    pos, in_macro = "rules", True
                elif ctx == "opaque":
                    pos, in_macro = "opaque_tt", True
                elif f.strs.inside(at):
                    pos, in_macro = "format_str", True
                else:
                    pos = "mod_decl"
            elif token.strip() == b"" or token == b"]":
                continue
            else:
                operators[(token.decode(errors="replace") if token in (b"?", b"[", b"*") else "binary/unary",
                           syms.info(sym)[1])] += 1
                continue
            refs.setdefault(at, sym)
            tgt = target(path, at, sym)
            group = POSITION_GROUP[pos]
            position[(group + (" (in macro)" if in_macro else ""), tgt)] += 1
            prev_sym = None
            if entry is not None and len(entry) > 2:
                prev = by_offset.get(entry[2])
                prev_sym = next((s for _, s, r in prev if not r & 1), None) if prev else None
            base = base_decision(pos, tgt, sym, prev_sym)
            if pos in ("opaque_tt", "rules"):
                s_dec, t_dec = "expansion", "expansion"
            elif in_macro:
                s_dec, t_dec = "expansion", base
            else:
                s_dec = t_dec = base
            dm = defined_in_macro(sym)
            if dm and s_dec != "types":
                s_dec = "expansion"
                if dm == "opaque" and t_dec != "types":
                    t_dec = "expansion"
            strict[(group, s_dec)] += 1
            transparent[(group, t_dec)] += 1
            decision_total["strict"][s_dec] += 1
            decision_total["transparent"][t_dec] += 1
            if tgt.startswith("local"):
                if in_macro:
                    hygiene["local_ref_in_" + ("format_str" if pos == "format_str" else
                                               "opaque_tt" if pos in ("opaque_tt", "rules") else "parsed_tt")] += 1
                else:
                    hygiene["local_ref_outside_macro"] += 1
                if dm:
                    hygiene["local_ref_to_binding_made_in_" + dm + "_tt"] += 1
            if pos == "macro_name" and not in_macro:
                macros[tgt] += 1

        # Identifiers rust-analyzer leaves without any occurrence.
        for entry in f.scan["idents"]:
            at, kind = entry[0], entry[1]
            if at in by_offset:
                continue
            text = f.ident(at)
            bare = kind[2:] if kind.startswith("m:") else kind
            if path not in occs:
                cls = "file_not_indexed"  # not in any crate's module tree under the indexed cfg
            elif f.cfg.inside(at):
                cls = "cfg_inactive"
            elif bare in ("attr", "derive"):
                cls = "builtin_attribute"
            elif text in PRIMITIVES:
                cls = "primitive_type"
            elif text in KEYWORDS:
                cls = "path_keyword"
            elif bare == "pat_ident":
                cls = "binding_not_indexed"
            elif kind.startswith("m:") or f.macro_context(at):
                cls = "in_macro"
            else:
                cls = "unresolved"
                if len(silent_samples[bare]) < 3:
                    line = bisect.bisect_right(f.starts, at)
                    silent_samples[bare].append(f"{path}:{line} {text}")
            silent[(cls, POSITION_GROUP.get(bare, bare))] += 1

        # Call sites, by callee resolution.
        for at, kind in f.scan["calls"]:
            in_macro = kind.startswith("m:")
            bare = kind[2:] if in_macro else kind
            sym = refs.get(at)
            if sym is None:
                cls = ("file_not_indexed" if path not in occs else
                       "cfg_inactive" if f.cfg.inside(at) else "unresolved")
                dec = "silent"
            else:
                tgt = target(path, at, sym)
                package, member, parent, kind_ = syms.info(sym)
                ext = tgt == "external"
                if bare == "call_method":
                    cls = {"inherent": "method_inherent", "trait_impl": "method_trait_impl",
                           "trait_item": "method_trait_decl"}.get(member, "method_other")
                    dec = "types"
                elif bare in ("call_other", "call_qself"):
                    cls, dec = "callee_expression", "types"
                elif tgt.startswith("local"):
                    cls, dec = "local_callable", "types"
                elif tgt.endswith("_nodef"):
                    cls, dec = "macro_generated_fn", "expansion"
                elif kind_ in KIND_CTOR or syms.names[sym].endswith("#"):
                    cls, dec = "constructor", ("syntax_extern" if ext else "syntax")
                elif member == "trait_impl":
                    cls, dec = "path_to_trait_impl_item", "types"
                elif member == "trait_item":
                    prev = f.idents.get(at)
                    prev_sym = refs.get(prev[2]) if prev is not None and len(prev) > 2 else None
                    if prev_sym is not None and syms.names[prev_sym] == parent:
                        cls, dec = "trait_qualified_path", ("syntax_extern" if ext else "syntax")
                    else:
                        cls, dec = "type_relative_trait_item", "types"
                elif member == "inherent":
                    cls, dec = "inherent_assoc_fn", ("syntax_extern" if ext else "syntax")
                else:
                    cls, dec = "free_fn", ("syntax_extern" if ext else "syntax")
            group = ("method" if bare == "call_method" else "path" if bare in ("call_path", "call_qself") else "other")
            calls[(group + (" (in macro)" if in_macro else ""), cls)] += 1
            call_decision["transparent"][dec] += 1
            call_decision["strict"]["expansion" if in_macro and dec in ("syntax", "syntax_extern") else dec] += 1

    resolved = sum(decision_total["strict"].values())
    total_calls = sum(call_decision["strict"].values())
    resolved_calls = total_calls - call_decision["strict"]["silent"]
    by_target = collections.Counter()
    for (_, tgt), n in position.items():
        by_target[tgt] += n
    for (cls, _), n in silent.items():
        if cls in ("unresolved", "in_macro"):
            by_target["unresolved (" + cls + ")"] += n
    return {
        "workspace_packages": workspace,
        "documents": len(docs),
        "files_scanned": len(files),
        "files_not_indexed": sorted(p for p in files if p not in occs),
        "files_syn_error": sorted(p for p, e in scan.items() if e["error"]),
        "reference_occurrences": resolved,
        "by_target": counter_dict(by_target),
        "decision": {mode: {"counts": counter_dict(c), "shares": {k: share(v, resolved) for k, v in sorted(c.items())}}
                     for mode, c in decision_total.items()},
        "position_by_target": counter_dict(position),
        "decision_by_position": {"strict": counter_dict(strict), "transparent": counter_dict(transparent)},
        "silent_identifiers": counter_dict(silent),
        "silent_unresolved_samples": {k: v for k, v in sorted(silent_samples.items())},
        "hygiene": counter_dict(hygiene),
        "macro_invocations_by_target": counter_dict(macros),
        "operator_dispatch_sites": counter_dict(operators),
        "calls": {
            "sites": total_calls,
            "resolved_sites": resolved_calls,
            "by_kind": counter_dict(calls),
            "decision": {mode: {"counts": counter_dict(c),
                                "shares_of_resolved": {k: share(v, resolved_calls) for k, v in sorted(c.items())
                                                       if k != "silent"}}
                         for mode, c in call_decision.items()},
        },
    }


# ---- values ----

LITERAL_CLASSES = {"literal", "literal_arith"}


def values(name: str) -> dict:
    _, _, scan = load(name)
    by_site = collections.Counter()
    by_class = collections.Counter()
    in_macro = collections.Counter()
    named: dict[str, list[dict]] = collections.defaultdict(list)
    sites = []
    for path in sorted(scan):
        for c in scan[path]["consts"]:
            if c["site"] == "const_param":
                by_site[("const_param", "declaration")] += 1
                continue
            sites.append(c)
            by_site[(c["site"], c["class"])] += 1
            by_class[c["class"]] += 1
            if c["macro"]:
                in_macro[c["class"]] += 1
            if c.get("name"):
                named[c["name"]].append(c)

    # Fixed point: a const_ref initializer is decidable when every referenced name is a standard numeric
    # constant or names only decidable constants of this corpus.
    decidable: dict[str, bool] = {}
    changed = True
    while changed:
        changed = False
        for n, cs in named.items():
            if decidable.get(n):
                continue
            ok = all(c["class"] in LITERAL_CLASSES or (c["class"] == "const_ref" and all(
                r in STD_CONSTS or decidable.get(r, False) for r in c["refs"])) for c in cs)
            if ok:
                decidable[n] = True
                changed = True
    following = collections.Counter()
    for c in sites:
        if c["class"] in LITERAL_CLASSES:
            following["literal_or_arith"] += 1
        elif c["class"] == "const_ref" and all(r in STD_CONSTS or decidable.get(r, False) for r in c["refs"]):
            following["const_ref_followed"] += 1
        elif c["class"] == "const_ref":
            following["const_ref_unfollowed"] += 1
        else:
            following["undecided:" + c["class"]] += 1
    total = len(sites)
    literal = by_class["literal"] + by_class["literal_arith"]
    return {
        "sites": total,
        "by_class": counter_dict(by_class),
        "by_site_and_class": counter_dict(by_site),
        "in_macro_token_trees": counter_dict(in_macro),
        "literal_arith_evaluator_share": share(literal, total),
        "with_const_following": counter_dict(following),
        "with_const_following_share": share(following["literal_or_arith"] + following["const_ref_followed"], total),
    }


# ---- suppression ----

def lint_catalogue() -> dict[str, str]:
    """Lint name (underscored, `clippy::` kept) -> rustc_lint | rustc_group | clippy_lint | clippy_group |
    clippy_renamed | clippy_deprecated, from clippy-driver 1.99.0 `-W help` and clippy's deprecated_lints.rs."""
    text = subprocess.run([str(BIN / "clippy-driver"), "-W", "help"], capture_output=True, text=True,
                          check=True).stdout
    catalogue: dict[str, str] = {}
    section = None
    for line in text.splitlines():
        if line.startswith("Lint checks provided by rustc"):
            section = "rustc_lint"
        elif line.startswith("Lint groups provided by rustc"):
            section = "rustc_group"
        elif line.startswith("Lint checks loaded by this crate"):
            section = "clippy_lint"
        elif line.startswith("Lint groups loaded by this crate"):
            section = "clippy_group"
        elif section and line.strip() and not line.strip().startswith(("name", "----")):
            word = line.split()[0]
            if re.fullmatch(r"[a-z0-9:_-]+", word):
                catalogue.setdefault(word.replace("-", "_"), section)
    # `-W help` lists `warnings` among the lints too; it is the group of every warn-level lint.
    catalogue["warnings"] = "rustc_group"
    source = (TOOLS / "rust-clippy" / "clippy_lints" / "src" / "deprecated_lints.rs").read_text()
    for block, cls in (("DEPRECATED", "clippy_deprecated"), ("RENAMED", "clippy_renamed")):
        body = source.split(block + "(", 1)[1].split("]}", 1)[0]
        for old in re.findall(r'\(\s*"([a-z0-9:_]+)"\s*,', body):
            catalogue.setdefault(old, cls)
    return catalogue


def suppression(name: str, catalogue: dict[str, str]) -> dict:
    _, _, scan = load(name)
    attrs = []
    for path in sorted(scan):
        attrs.extend(scan[path]["attrs"])
    level = collections.Counter(a["level"] for a in attrs)
    placement = collections.Counter({k: 0 for k in (
        "crate_level_inner", "inner_non_crate", "outer", "under_cfg_attr", "with_reason",
        "inside_macro_invocation_tt", "inside_macro_rules_body", "on_macro_invocation")})
    target = collections.Counter()
    namespace = collections.Counter()
    names_ = collections.Counter()
    for a in attrs:
        if a["target"] == "crate":
            placement["crate_level_inner"] += 1
        elif a["inner"]:
            placement["inner_non_crate"] += 1
        else:
            placement["outer"] += 1
        if a["cfg_attr"] is not None:
            placement["under_cfg_attr"] += 1
        if a["reason"]:
            placement["with_reason"] += 1
        if a["in_macro_tt"]:
            placement["inside_macro_invocation_tt"] += 1
        if a["in_macro_rules"]:
            placement["inside_macro_rules_body"] += 1
        if a["target"] == "macro_call":
            placement["on_macro_invocation"] += 1
        target[a["target"]] += 1
        for lint in a["lints"]:
            key = lint.replace("-", "_")
            cls = "rustdoc_lint" if key.startswith("rustdoc::") else catalogue.get(key, "unknown")
            namespace[cls] += 1
            names_[key] += 1
    cfg_preds = collections.Counter(re.sub(r"\s+", "", a["cfg_attr"]) for a in attrs if a["cfg_attr"] is not None)
    return {
        "attributes": len(attrs),
        "lint_names": sum(namespace.values()),
        "by_level": counter_dict(level),
        "placement": counter_dict(placement),
        "by_target": counter_dict(target),
        "by_lint_namespace": counter_dict(namespace),
        "cfg_attr_predicates": counter_dict(cfg_preds),
        "top_lints": dict(list(counter_dict(names_).items())[:25]),
        "unknown_lints": sorted(k for k in names_ if catalogue.get(k) is None and not k.startswith("rustdoc::")),
    }


def header() -> dict:
    return {
        "generator": "rerail-fixtures/rust/feasibility/generate.py",
        "tools": {**VERSIONS, "scanner": SCANNER, "rust-clippy": CLIPPY_COMMIT, "rust-analyzer-config": RA_CONFIG},
        "corpora": {n: {"repository": c["repository"], "commit": c["commit"]} for n, c in CORPORA.items()},
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reuse", action="store_true")
    args = parser.parse_args()
    check_pins()
    tool = scanner()
    for name in CORPORA:
        raw = RAW / "out" / (name + ".scan.jsonl")
        if not (args.reuse and raw.exists() and (RAW / "out" / (name + ".scip.jsonl")).exists()):
            produce(name, tool)
    catalogue = lint_catalogue()
    outputs = {
        "names.json": {n: names(n) for n in CORPORA},
        "values.json": {n: values(n) for n in CORPORA},
        "suppression.json": {n: suppression(n, catalogue) for n in CORPORA},
    }
    for file, body in outputs.items():
        (HERE / file).write_text(json.dumps({**header(), "corpora_results": body}, indent=1, sort_keys=False) + "\n")


if __name__ == "__main__":
    main()
