"""The CPython oracle of `cpython::parity` (test only): CPython's own facts for each file named on stdin.

Run as `python3 -I -S -c <this> <mode>`: stdin is one path per line, stdout one JSON object per file. Never imports or
runs the files. Modes:

- `syntax`: {"p", "e"}: e is null where `ast.parse` accepts the text, else [lineno, msg].
- `ast`: {"p", "n"}: every located node of the file's tree (none where it does not parse) but an f-string's own pieces
  (a JoinedStr's values and their format specs; the expressions of its fields are kept), each
  [start, end, type, unparse, dump_sha256, body_sha256, signature_sha256]: start and end are UTF-8 byte offsets,
  unparse is `ast.unparse` of an expression, a keyword, an arg or an alias (null for another node or where it raises;
  {"surrogates": its ascii} where it holds a lone surrogate, which UTF-8 cannot: a format spec's text is written as it
  is), dump_sha256 the sha256 of `ast.dump(node, include_attributes=False)` (null where it raises), body_sha256 and
  signature_sha256 a def's as W's python_ast worker hashed them (null but for a def).
- `names`: {"p", "n"}: each Name id, Attribute attr, def/class name and arg, [start, end, text].
- `literals`: {"p", "n"}: each expression but an f-string's own pieces, [start, end, type, literal_json] (literal_json as
  in `ast`, null where literal_eval or json.dumps raises).
- `annotations`: {"p", "n"}: each str Constant, [start, end, parsed, names, annotation]: parsed whether
  `ast.parse(value, mode="eval")` accepts it, names the sorted Name ids and Attribute attrs of its tree, annotation
  whether it stands at an annotation (field annotation or returns, or anywhere below one).
"""
import ast
import hashlib
import json
import multiprocessing
import re
import sys

MODE = sys.argv[1]
LINES = re.compile(rb"\r\n?|\n")
SURROGATE = re.compile(r"\\ud[89a-f]")


def read(path):
    with open(path, "rb") as f:
        data = f.read()
    return data.decode("utf-8")


def offsets_of(source_bytes):
    return [0, *(m.end() for m in LINES.finditer(source_bytes))]


def span(node, offsets):
    return offsets[node.lineno - 1] + node.col_offset, offsets[node.end_lineno - 1] + node.end_col_offset


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def dump(node):
    return ast.dump(node, include_attributes=False)


def def_hashes(node):
    try:
        body = sha(dump(ast.Module(body=node.body, type_ignores=[])))
    except Exception:
        body = None
    try:
        sig = dump(node.args)
        sig += "|" + (dump(node.returns) if node.returns is not None else "None")
        sig += "|" + dump(ast.Tuple(elts=node.type_params))
        sig = sha(sig)
    except Exception:
        sig = None
    return [body, sig]


def literal_json(node):
    try:
        return json.dumps(ast.literal_eval(node), sort_keys=True, separators=(",", ":"))
    except Exception:
        return None


def one(path):
    try:
        source = read(path)
    except (UnicodeDecodeError, OSError):
        return None
    if MODE == "syntax":
        try:
            ast.parse(source)
            return {"p": path, "e": None}
        except SyntaxError as e:
            return {"p": path, "e": [e.lineno, e.msg]}
        except (RecursionError, MemoryError, ValueError) as e:
            return {"p": path, "e": [None, type(e).__name__]}
    try:
        tree = ast.parse(source)
    except Exception:
        return {"p": path, "n": None}
    offsets = offsets_of(source.encode("utf-8"))
    out = []
    if MODE == "ast":
        pieces = set()
        for n in ast.walk(tree):
            if isinstance(n, ast.JoinedStr):
                for c in n.values:
                    pieces.add(id(c))
                    if isinstance(c, ast.FormattedValue) and c.format_spec is not None:
                        pieces.add(id(c.format_spec))
        for node in ast.walk(tree):
            if not hasattr(node, "lineno") or node.end_lineno is None or id(node) in pieces:
                continue
            s, e = span(node, offsets)
            text = None
            if isinstance(node, (ast.expr, ast.keyword, ast.arg, ast.alias)):
                try:
                    text = ast.unparse(node)
                except Exception:
                    text = None
            try:
                digest = sha(dump(node))
            except Exception:
                digest = None
            body, sig = def_hashes(node) if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) else (None, None)
            out.append([s, e, type(node).__name__, text, digest, body, sig])
    elif MODE == "literals":
        pieces = {id(c) for n in ast.walk(tree) if isinstance(n, ast.JoinedStr) for c in n.values}
        for node in ast.walk(tree):
            if not isinstance(node, ast.expr) or id(node) in pieces:
                continue
            s, e = span(node, offsets)
            out.append([s, e, type(node).__name__, literal_json(node)])
    elif MODE == "names":
        for node in ast.walk(tree):
            if isinstance(node, ast.Name):
                text = node.id
            elif isinstance(node, ast.Attribute):
                text = node.attr
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                text = node.name
            elif isinstance(node, ast.arg):
                text = node.arg
            else:
                continue
            s, e = span(node, offsets)
            out.append([s, e, type(node).__name__, text])
    elif MODE == "annotations":
        parents = {}
        for parent in ast.walk(tree):
            for field, value in ast.iter_fields(parent):
                for child in value if isinstance(value, list) else [value]:
                    if isinstance(child, ast.AST):
                        parents[id(child)] = (parent, field)
        roots = [n for n in ast.walk(tree) if parents.get(id(n), (None, ""))[1] in ("annotation", "returns")]
        annotated = {id(node) for root in roots for node in ast.walk(root)}
        for node in ast.walk(tree):
            if not (isinstance(node, ast.Constant) and isinstance(node.value, str)) or parents.get(id(node), (None, ""))[0].__class__ is ast.JoinedStr:
                continue
            try:
                inner = ast.parse(node.value, mode="eval").body
                parsed = True
                names = sorted({c.id if isinstance(c, ast.Name) else c.attr for c in ast.walk(inner) if isinstance(c, (ast.Name, ast.Attribute))})
            except (SyntaxError, ValueError, RecursionError, MemoryError):
                parsed, names = False, []
            s, e = span(node, offsets)
            out.append([s, e, parsed, names, id(node) in annotated])
    return {"p": path, "n": out}


def utf8(value):
    """`value` with each str that UTF-8 cannot hold (a lone surrogate: `ast.unparse` writes a format spec's text as it
    is) as {"surrogates": its `ascii`}, which no str of the engine is."""
    if isinstance(value, str):
        try:
            value.encode("utf-8")
            return value
        except UnicodeEncodeError:
            return {"surrogates": ascii(value)}
    if isinstance(value, list):
        return [utf8(v) for v in value]
    if isinstance(value, dict):
        return {k: utf8(v) for k, v in value.items()}
    return value


def row_line(path):
    """The JSON line of `path`'s row (ASCII), none where it is not read."""
    row = one(path)
    if row is None:
        return None
    text = json.dumps(row, ensure_ascii=True)
    # A surrogate escape: of a pair (which UTF-8 holds) or a lone one (which it does not).
    if SURROGATE.search(text):
        text = json.dumps(utf8(row), ensure_ascii=True)
    return text


def main():
    paths = [line.rstrip("\n") for line in sys.stdin if line.strip()]
    with multiprocessing.get_context("fork").Pool() as pool:
        for text in pool.imap(row_line, paths, chunksize=16):
            if text is not None:
                sys.stdout.write(text + "\n")


sys.setrecursionlimit(10000)
main()
