"""Records Jedi's call targets over sampled Python corpora for `benchmark_python_calls_corpus`
(the T3 differential of Rerail's `calls`). Offline, not in CI, with Jedi 0.19.2 installed in the
interpreter that runs it:

    python3 generate_corpus.py <output.json> <name>=<checkout>:<root>[,<root>...]:<path>[,<path>...] ...

Each `<root>` is a directory of the checkout that holds top-level packages or modules (it goes on
Jedi's sys.path, as on the program's); each `<path>` a directory of the checkout whose .py files
are the corpus. Rerail links every one of those files; this script samples one file in SAMPLE of
them (by the SHA-256 of its path from the checkout, so the sample is stable) and, for each call
whose callee is a name or an attribute (`f(...)`, `a.b.f(...)`), asks Jedi where that name is
defined (`Script.goto` at the callee's last name, imports followed). `goto` is used rather than
`infer`: `infer` reads a classmethod as `classmethod.__get__` and prefers Jedi's bundled Django
stubs to the corpus's own source. A name `goto` finds bound by an assignment, a parameter or an
import Jedi cannot follow is `other`: Jedi does not decide what it holds. Files CPython cannot
parse are excluded. Writes {"jedi", "python", "sample", "corpora": [{"name", "checkout", "roots", "paths",
"files", "sampled", "excluded", "records": {file: [[start, end, line, targets], ...]}}]}: paths
from the checkout, byte offsets, and each target one of
- ["def", file, line]: a function or method of the corpus, at its `def` line;
- ["class", file, line]: a class of the corpus, at its `class` line;
- ["external", full name]: a function or class outside the corpus (the standard library, builtins);
- ["other", type]: anything else Jedi names (a variable, an instance, a module, a parameter).
A call Jedi names nothing for has no targets."""

from __future__ import annotations

import ast
import hashlib
import json
import os
import pathlib
import platform
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor

import jedi

JEDI = "0.19.2"
# 1: every file.
SAMPLE = 1


def private_cache() -> None:
    """Jedi's on-disk cache is shared by default; workers writing it at once corrupt it."""
    jedi.settings.cache_directory = os.path.join(tempfile.gettempdir(), f"rerail-jedi-{os.getpid()}")


def sampled(relative: str) -> bool:
    return hashlib.sha256(relative.encode()).digest()[0] % SAMPLE == 0


def python_files(checkout: pathlib.Path, directory: pathlib.Path) -> list[str]:
    relatives = (path.relative_to(checkout) for path in directory.rglob("*.py"))
    return sorted(
        relative.as_posix() for relative in relatives
        if not any(part.startswith(".") or part in ("node_modules", "__pycache__") for part in relative.parts)
    )


# Jedi ships stubs for some third-party packages (Django among them); where `goto` lands in one,
# Jedi names its stub, not the program's code the corpus holds: undecided.
BUNDLED_STUBS = pathlib.Path(jedi.__file__).resolve().parent / "third_party"
STANDARD_STUBS = BUNDLED_STUBS / "typeshed" / "stdlib"


def target(name: jedi.api.classes.Name, checkout: pathlib.Path) -> list[object]:
    module = None if name.module_path is None else pathlib.Path(name.module_path).resolve()
    if module is not None and module.is_relative_to(BUNDLED_STUBS) and not module.is_relative_to(STANDARD_STUBS):
        return ["other", "bundled-stub"]
    inside = module is not None and module.is_relative_to(checkout)
    if name.type in ("function", "class") and inside and name.line is not None:
        return ["def" if name.type == "function" else "class", module.relative_to(checkout).as_posix(), name.line]
    if name.type in ("function", "class"):
        return ["external", name.full_name or name.name]
    return ["other", name.type]


def record_file(job: tuple[str, str, list[str]]) -> tuple[str, list[list[object]]]:
    checkout_text, relative, roots = job
    checkout = pathlib.Path(checkout_text)
    project = jedi.Project(checkout, added_sys_path=roots, smart_sys_path=False, load_unsafe_extensions=False)
    data = (checkout / relative).read_bytes()
    code = data.decode("utf-8")
    lines = data.split(b"\n")
    starts = [0]
    for line in lines[:-1]:
        starts.append(starts[-1] + len(line) + 1)
    script = jedi.Script(code, path=checkout / relative, project=project)
    records = []
    for node in ast.walk(ast.parse(data)):
        if not isinstance(node, ast.Call):
            continue
        callee = node.func
        if isinstance(callee, ast.Name):
            line, column = callee.lineno, callee.col_offset
        elif isinstance(callee, ast.Attribute):
            line, column = callee.end_lineno, callee.end_col_offset - len(callee.attr.encode())
        else:
            continue
        # Jedi's columns count code points; ast's count UTF-8 bytes.
        characters = len(lines[line - 1][:column].decode("utf-8"))
        try:
            names = script.goto(line, characters, follow_imports=True)
        except Exception as error:  # Jedi's own failures leave the call undecided.
            names = []
            print(f"{relative}:{line}: jedi {type(error).__name__}", file=sys.stderr)
        targets = sorted({json.dumps(target(name, checkout)) for name in names})
        start = starts[node.lineno - 1] + node.col_offset
        end = starts[node.end_lineno - 1] + node.end_col_offset
        records.append([start, end, node.lineno, [json.loads(text) for text in targets]])
    records.sort()
    return relative, records


def main() -> None:
    if jedi.__version__ != JEDI:
        sys.exit(f"generate_corpus.py runs Jedi {JEDI}, not {jedi.__version__}")
    output = pathlib.Path(sys.argv[1])
    corpora = []
    for argument in sys.argv[2:]:
        name, rest = argument.split("=", 1)
        checkout_text, roots_text, paths_text = rest.split(":", 2)
        checkout = pathlib.Path(checkout_text).resolve()
        roots = [str(checkout / root) for root in roots_text.split(",")]
        paths = paths_text.split(",")
        files = [relative for path in paths for relative in python_files(checkout, checkout / path)]
        excluded = []
        for relative in files:
            try:
                ast.parse((checkout / relative).read_bytes())
            except (SyntaxError, ValueError):
                excluded.append(relative)
        chosen = [relative for relative in files if relative not in excluded and sampled(relative)]
        with ProcessPoolExecutor(max_workers=6, initializer=private_cache) as pool:
            records = dict(pool.map(record_file, [(str(checkout), relative, roots) for relative in chosen]))
        corpora.append({
            "name": name, "checkout": str(checkout), "roots": roots, "paths": paths, "files": len(files),
            "sampled": len(chosen), "excluded": excluded, "records": records,
        })
    output.write_text(json.dumps({
        "jedi": jedi.__version__, "python": platform.python_version(), "sample": SAMPLE, "corpora": corpora,
    }))


if __name__ == "__main__":
    main()
