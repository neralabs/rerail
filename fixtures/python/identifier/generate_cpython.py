"""Writes cpython.expected.json: what CPython 3.13 makes of identifiers written with compatibility characters (PEP 3131
normalizes every identifier token with NFKC while string contents stay as written). Run offline, not in CI:
python3.13 generate_cpython.py

- `modules`: for each source, every NAME token that is no keyword with the identifier CPython's own parser reads it as
  (`ast.parse(token, mode="eval")`), and the module-level loads no module binding (symtable: assigned or imported) or
  builtin defines. Loads inside functions, lambdas, classes and comprehensions are not judged, nor is the order of
  binding and use: each source binds before it reads. `run` is what executing the source raises, or `ok`.
- `structures`: whether two expressions parse to the same AST (`ast.dump`, positions aside).
- `runs`: what a snippet evaluates `result` to, or the exception it raises: keyword arguments, attributes, imports,
  builtins, `str.format` fields, `getattr` strings and `__all__` entries.
Offsets are UTF-8 byte offsets into the source.
"""

from __future__ import annotations

import ast
import builtins
import io
import json
import keyword
import pathlib
import symtable
import sys
import tokenize
import types
import unicodedata
from contextlib import redirect_stdout

HERE: pathlib.Path = pathlib.Path(__file__).parent
F821_28: pathlib.Path = HERE.parent / "literal" / "ruff" / "F821_28.py"

MODULES: list[tuple[str, str]] = [
    ("F821_28", F821_28.read_text()),
    (
        "bindings",
        "𝒞 = 1\n"
        "print(C, 𝑪, Ｃ)\n"
        "ﬁle = 2\n"
        "print(file)\n"
        "𝐟𝐨𝐫𝐦𝐚𝐭 = 3\n"
        "print(format)\n"
        "print(𝐥𝐞𝐧('ab'))\n"
        "print(𝒟)\n",
    ),
    (
        "imports",
        "import 𝐨𝐬\n"
        "from 𝐨𝐬 import 𝐬𝐞𝐩 as 𝒮\n"
        "print(os.sep, os.𝐩𝐚𝐭𝐡, S, sep)\n",
    ),
    (
        "definitions",
        "def 𝐟(𝐤=1):\n"
        "    return k\n"
        "class 𝒜:\n"
        "    pass\n"
        "print(f(k=2), A, 𝒜, 𝐀)\n",
    ),
    (
        "dunder_all",
        "𝒞 = 1\n"
        "__𝐚𝐥𝐥__ = ['𝒞', 'C']\n"
        "print(__all__)\n",
    ),
]

STRUCTURES: list[tuple[str, str]] = [
    ("𝒞 + 1", "C + 1"),
    ("𝒞 + 1", "𝓒 + 1"),
    ("𝒞 + 1", "D + 1"),
    ("x.𝐚𝐭𝐭𝐫", "x.attr"),
    ("f(𝐤=1)", "f(k=1)"),
    ("lambda 𝐱: x", "lambda x: 𝐱"),
    ("'𝒞'", "'C'"),
    ("{𝒞: 1}", "{C: 1}"),
]

RUNS: list[tuple[str, str]] = [
    ("keyword", "def f(k):\n    return k\nresult = f(𝐤=1)\n"),
    ("keyword_twice", "def f(k):\n    return k\nresult = f(k=1, 𝐤=2)\n"),
    ("keyword_splat", "def f(k=0):\n    return k\nresult = f(**{'𝐤': 1})\n"),
    ("attribute", "class A:\n    C = 1\nresult = A.𝒞\n"),
    ("getattr", "class A:\n    C = 1\nresult = getattr(A, '𝒞', 'missing')\n"),
    ("import", "import 𝐨𝐬\nresult = os.sep == 𝐨𝐬.sep\n"),
    ("builtin", "result = 𝐥𝐞𝐧('ab')\n"),
    ("format_keyword", "result = '{C}'.format(𝒞=1)\n"),
    ("format_field", "result = '{𝒞}'.format(C=1)\n"),
    ("global", "def f():\n    global 𝒞\n    𝒞 = 1\nf()\nresult = C\n"),
    ("nonlocal", "def f():\n    C = 1\n    def g():\n        nonlocal 𝒞\n        𝒞 = 2\n    g()\n    return C\nresult = f()\n"),
    ("star_import", "import nfkc_module\nresult = nfkc_module.__all__\nfrom nfkc_module import *\n"),
]


def byte_offsets(source: str) -> list[int]:
    """The byte offset each line starts at (1-based line numbers index it)."""
    offsets: list[int] = [0, 0]
    for line in source.splitlines(keepends=True):
        offsets.append(offsets[-1] + len(line.encode()))
    return offsets


def parser_name(token: str) -> str:
    node = ast.parse(token, mode="eval").body
    if not isinstance(node, ast.Name):
        raise SystemExit(f"{token!r} is no name")
    return node.id


def identifiers(source: str) -> list[dict[str, object]]:
    lines: list[int] = byte_offsets(source)
    found: list[dict[str, object]] = []
    text: list[str] = source.splitlines(keepends=True)
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
        if token.type != tokenize.NAME or keyword.iskeyword(token.string) or keyword.issoftkeyword(token.string):
            continue
        (row, column), (_, end_column) = token.start, token.end
        start: int = lines[row] + len(text[row - 1][:column].encode())
        end: int = lines[row] + len(text[row - 1][:end_column].encode())
        found.append({"start": start, "end": end, "text": token.string, "name": parser_name(token.string)})
    return found


class ModuleLoads(ast.NodeVisitor):
    """Name loads of the module's own scope."""

    def __init__(self) -> None:
        self.loads: list[ast.Name] = []

    def visit_Name(self, node: ast.Name) -> None:
        if isinstance(node.ctx, ast.Load):
            self.loads.append(node)

    def skip(self, node: ast.AST) -> None:
        del node

    visit_FunctionDef = visit_AsyncFunctionDef = visit_Lambda = visit_ClassDef = skip
    visit_ListComp = visit_SetComp = visit_DictComp = visit_GeneratorExp = skip


def undefined(source: str) -> list[dict[str, object]]:
    module = symtable.symtable(source, "module", "exec")
    bound: set[str] = {symbol.get_name() for symbol in module.get_symbols() if symbol.is_assigned() or symbol.is_imported()}
    visitor = ModuleLoads()
    visitor.visit(ast.parse(source))
    lines: list[int] = byte_offsets(source)
    encoded: bytes = source.encode()
    found: list[dict[str, object]] = []
    for node in visitor.loads:
        if node.id in bound or hasattr(builtins, node.id):
            continue
        start: int = lines[node.lineno] + node.col_offset
        end: int = lines[node.end_lineno or node.lineno] + (node.end_col_offset or 0)
        found.append({"start": start, "end": end, "text": encoded[start:end].decode(), "name": node.id})
    return sorted(found, key=lambda item: int(str(item["start"])))


def outcome(source: str) -> dict[str, str]:
    namespace: dict[str, object] = {"__name__": "nfkc_case"}
    try:
        with redirect_stdout(io.StringIO()):
            exec(compile(source, "<case>", "exec"), namespace)  # noqa: S102 - the oracle runs its own cases
    except Exception as error:  # noqa: BLE001 - the oracle records every failure by type and message
        return {"error": f"{type(error).__name__}: {error}"}
    if "result" in namespace:
        return {"result": repr(namespace["result"])}
    return {"result": "ok"}


def install_star_module() -> None:
    """`nfkc_module`, whose `__all__` lists `𝒞` (as written) and `C`: it binds only `C`."""
    module = types.ModuleType("nfkc_module")
    exec("𝒞 = 1\n__all__ = ['C', '𝒞']\n", module.__dict__)  # noqa: S102 - the oracle runs its own cases
    sys.modules["nfkc_module"] = module


def main() -> None:
    if sys.version_info[:2] != (3, 13):
        raise SystemExit("run with CPython 3.13")
    install_star_module()
    expected: dict[str, object] = {
        "python": sys.version.split()[0],
        "unicode": unicodedata.unidata_version,
        "modules": [
            {
                "name": name,
                "source": source,
                "identifiers": identifiers(source),
                "undefined": undefined(source),
                "run": outcome(source),
            }
            for name, source in MODULES
        ],
        "structures": [
            {
                "left": left,
                "right": right,
                "equal": ast.dump(ast.parse(left, mode="eval")) == ast.dump(ast.parse(right, mode="eval")),
            }
            for left, right in STRUCTURES
        ],
        "runs": [{"name": name, "source": source, "outcome": outcome(source)} for name, source in RUNS],
    }
    (HERE / "cpython.expected.json").write_text(json.dumps(expected, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
