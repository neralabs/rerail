"""How the port readiness gates read Python, run in the pinned environment of conformance/recorders/port-readiness/runtimes/python.

One JSON request on stdin, one JSON response on stdout:
- {"op": "keywords"}: the language's keywords, soft keywords included.
- {"op": "words", "texts": [...]}: per text, every word of every token but numbers, as the TypeScript adapter scans.
- {"op": "literals", "files": [...]}: per file, the names its code checks (identifier-shaped string literals, the
  segments of qualified names and paths, quoted words in strings, words of regexes it compiles), the regexes
  themselves, and its imports.
- {"op": "verdicts", "files": [{"path", "source"}]}: per file, syntax and grammar errors from CPython (the reference
  implementation, which also checks what Python forbids after parsing) and syntax errors from libcst (an independent
  parser with no grammar checks).
"""

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


def words(text):
    found = []
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


def docstrings(tree):
    """The string nodes that document a module, class or function rather than being checked."""
    found = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
            first = node.body[0]
            if isinstance(first, ast.Expr) and isinstance(first.value, ast.Constant) and isinstance(first.value.value, str):
                found.add(id(first.value))
    return found


def literals(path):
    with open(path, encoding="utf-8") as handle:
        tree = ast.parse(handle.read(), filename=path)
    skip = docstrings(tree)
    names, regexes, imports = set(), [], set()
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


def first_line(error):
    return str(error).splitlines()[0] if str(error) else type(error).__name__


def cpython_verdict(path, source):
    try:
        tree = ast.parse(source, filename=path)
    except SyntaxError as error:
        return {"syntax": [f"{type(error).__name__}: {error.msg} (line {error.lineno})"], "grammar": []}
    try:
        # Compiling the tree runs the checks CPython makes after parsing (scopes, `return` outside a function, ...).
        compile(tree, path, "exec", dont_inherit=True)
    except SyntaxError as error:
        return {"syntax": [], "grammar": [f"{type(error).__name__}: {error.msg} (line {error.lineno})"]}
    return {"syntax": [], "grammar": []}


def libcst_verdict(source):
    import libcst

    try:
        libcst.parse_module(source)
    except libcst.ParserSyntaxError as error:
        return {"syntax": [f"{first_line(error)} (line {error.raw_line})"], "grammar": None}
    except Exception as error:  # libcst raises plain exceptions for some source it cannot represent
        return {"syntax": [first_line(error)], "grammar": None}
    # libcst builds a concrete syntax tree and checks nothing CPython checks after parsing.
    return {"syntax": [], "grammar": None}


def verdicts(path, source):
    from importlib.metadata import version as installed

    version = ".".join(str(part) for part in sys.version_info[:3])
    return [
        {"parser": f"cpython {version}", **cpython_verdict(path, source)},
        {"parser": f"libcst {installed('libcst')}", **libcst_verdict(source)},
    ]


def main():
    request = json.load(sys.stdin)
    op = request["op"]
    if op == "keywords":
        response = sorted(set(keyword.kwlist) | set(keyword.softkwlist))
    elif op == "words":
        response = [words(text) for text in request["texts"]]
    elif op == "literals":
        response = [literals(path) for path in request["files"]]
    elif op == "verdicts":
        response = [verdicts(file["path"], file["source"]) for file in request["files"]]
    else:
        raise SystemExit(f"unknown op {op}")
    json.dump(response, sys.stdout)


if __name__ == "__main__":
    main()
