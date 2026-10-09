"""How the source-facts recorders read Python, under CPython 3.14 (python-toolkit.ts). Ported from
nera-rule-validation's src/validation/languages/python_toolkit.py.

One JSON request on stdin, one JSON response on stdout:
- {"op": "keywords"}: the language's keywords, soft keywords included.
- {"op": "words", "texts": [...]}: every word of every token but numbers, over all the texts, sorted and distinct.
- {"op": "literals", "files": [...]}: per file, the names its code checks (identifier-shaped string literals, the
  segments of qualified names and paths, quoted words in strings, words of regexes it compiles), the regexes
  themselves, and its imports.
"""

from __future__ import annotations

import ast
import io
import json
import keyword
import re
import sys
import tokenize

WORD = re.compile(r"[A-Za-z_][\w]*")
IDENTIFIER = re.compile(r"^[A-Za-z_]\w*$")
# A qualified name or path that upstream matches by prefix or suffix: `pickle.loads`, `/var/tmp`, `.run`.
SEGMENTED = re.compile(r"^[./]?[A-Za-z_]\w*(?:[./][A-Za-z_]\w*)*$")
QUOTED = re.compile(r"[\"'`]([A-Za-z_]\w*)[\"'`]")
NUMBER = re.compile(r"\b(?:0[xob][0-9a-f_]+|\d[\d_]*(?:\.\d*)?(?:e[+-]?\d+)?j?)\b", re.IGNORECASE)
REGEX_FUNCTIONS = {"compile", "match", "search", "fullmatch", "findall", "finditer", "sub", "subn", "split"}


def words(text: str) -> list[str]:
    found: list[str] = []
    try:
        for token in tokenize.generate_tokens(io.StringIO(text).readline):
            if token.type == tokenize.NUMBER:
                continue
            if token.type == tokenize.NAME:
                found.append(token.string)
            else:
                found.extend(WORD.findall(token.string))
        return found
    except (tokenize.TokenError, SyntaxError):
        # Upstream test files may be deliberately broken. The tokenizer stops there, so read the whole text as words,
        # leaving out number literals as the tokenizer would.
        return WORD.findall(NUMBER.sub(" ", text))


def docstrings(tree: ast.AST) -> set[int]:
    """The string nodes that document a module, class or function rather than being checked."""
    found: set[int] = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
            first = node.body[0]
            if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant) and isinstance(first.value.value, str):
                found.add(id(first.value))
    return found


def literals(path: str) -> dict:
    with open(path, encoding="utf-8") as handle:
        tree = ast.parse(handle.read(), filename=path)
    skip = docstrings(tree)
    names: set[str] = set()
    regexes: list[str] = []
    imports: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Constant) and isinstance(node.value, str) and id(node) not in skip:
            text = node.value
            if IDENTIFIER.match(text):
                names.add(text)
            elif SEGMENTED.match(text):
                names.update(segment for segment in re.split(r"[./]", text) if segment)
            names.update(QUOTED.findall(text))
        elif isinstance(node, ast.Call) and node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
            function = node.func
            name = function.attr if isinstance(function, ast.Attribute) else function.id if isinstance(function, ast.Name) else None
            receiver = function.value.id if isinstance(function, ast.Attribute) and isinstance(function.value, ast.Name) else None
            if name in REGEX_FUNCTIONS and receiver == "re":
                regexes.append(node.args[0].value)
        elif isinstance(node, ast.Import):
            imports.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module is not None and node.level == 0:
            imports.add(node.module)
            imports.update(f"{node.module}.{alias.name}" for alias in node.names)
    for pattern in regexes:
        names.update(WORD.findall(re.sub(r"\\[A-Za-z]", " ", pattern)))
    return {"names": sorted(names), "regexes": regexes, "imports": sorted(imports)}


def main() -> None:
    # Keywords and the tokenizer change between minor versions; the facts are recorded under one.
    if sys.version_info[:2] != (3, 14):
        raise SystemExit(f"run under CPython 3.14, not {sys.version.split()[0]}")
    request = json.load(sys.stdin)
    op = request["op"]
    if op == "keywords":
        response: object = sorted(set(keyword.kwlist) | set(keyword.softkwlist))
    elif op == "words":
        response = sorted({word for text in request["texts"] for word in words(text)})
    elif op == "literals":
        response = [literals(path) for path in request["files"]]
    else:
        raise SystemExit(f"unknown op {op}")
    json.dump(response, sys.stdout)


if __name__ == "__main__":
    main()
