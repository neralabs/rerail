"""Canonical CPython AST facts. Source text is parsed and is never executed."""
import ast
import bisect
import hashlib
import json
import re
import sys
import unicodedata

if sys.version_info[:2] != (3, 13):
    raise RuntimeError("Canonical Python AST facts require CPython 3.13")
request = json.load(sys.stdin)
if request.get("format") != "rerail.python-ast/v1":
    raise ValueError("Unknown Python AST batch format")
if sum(len(document["source"].encode("utf-8")) for document in request["documents"]) > 8 * 1024 * 1024:
    raise ValueError("Python AST batch exceeds its source capacity")

KINDS = {
    "function_definition": (ast.FunctionDef, ast.AsyncFunctionDef),
    "class_definition": (ast.ClassDef,),
    "assignment": (ast.Assign, ast.AnnAssign),
    "identifier": (ast.Name,),
    "attribute": (ast.Attribute,),
    "call": (ast.Call,),
    "string": (ast.Constant, ast.JoinedStr),
    "concatenated_string": (ast.Constant, ast.JoinedStr),
    "integer": (ast.Constant,),
    "float": (ast.Constant,),
    "tuple": (ast.Tuple,),
    "list": (ast.List,),
    "dictionary": (ast.Dict,),
    "subscript": (ast.Subscript,),
    "type": (ast.expr,),
    "import_from_statement": (ast.ImportFrom,),
    "true": (ast.Constant,),
    "false": (ast.Constant,),
    "none": (ast.Constant,),
}
output = []
output_bytes = 0
module_walks = 0
parses = 0
for document in request["documents"]:
    source = document["source"]
    def text_only(item):
        return item['fields']==['column'] or item['kind'] in {'identifier','dotted_name','relative_import'} and item['fields']==['identifier']
    tree = None
    syntax_error = None
    if not all(text_only(item) for item in document['requests']):
        parses += 1
        try:
            tree = ast.parse(source)
        except SyntaxError as error:
            if not all(item['kind'] == 'module' and item['fields'] == ['syntax_error']
                       for item in document['requests']):
                raise
            syntax_error = {name: getattr(error, name) for name in (
                'msg', 'lineno', 'offset', 'text', 'end_lineno', 'end_offset')}
    source_bytes=source.encode("utf-8")
    offsets = [0, *(match.end() for match in re.finditer(rb"\r\n?|\n",source_bytes))]
    by_span = {}
    by_start = {}
    needed_types=tuple({kind for item in document["requests"] if item["kind"]!="module" and not text_only(item) for kind in KINDS.get(item["kind"],())})
    module_walks += bool(needed_types)
    for node in ast.walk(tree) if needed_types else ():
        if not isinstance(node,needed_types) or not hasattr(node, "lineno"):
            continue
        span = (offsets[node.lineno - 1] + node.col_offset,
                offsets[node.end_lineno - 1] + node.end_col_offset)
        by_span.setdefault(span, []).append(node)
        by_start.setdefault(span[0], []).append((span[1],node))
    rows = []
    for item in document["requests"]:
        kind = item["kind"]
        if item["fields"] == ["column"]:
            value=str(item["start"]-offsets[bisect.bisect_right(offsets,item["start"])-1])
            output_bytes+=len(value)
            if output_bytes>64*1024*1024:raise ValueError("Python AST batch exceeds its output capacity")
            rows.append({"column":value})
            continue
        if kind in {"identifier","dotted_name","relative_import"} and item["fields"] == ["identifier"]:
            text=source_bytes[item["start"]:item["end"]].decode("utf-8")
            value=unicodedata.normalize("NFKC",re.sub(r"\\\r?\n|\s+","",text))
            output_bytes+=len(value.encode("utf-8"))
            if output_bytes>64*1024*1024:raise ValueError("Python AST batch exceeds its output capacity")
            rows.append({"identifier":value})
            continue
        if kind == "module":
            node = tree
        else:
            candidates = by_span.get((item["start"], item["end"]), ())
            allowed = KINDS.get(kind)
            if allowed is None:
                raise ValueError("Unsupported canonical AST root kind: " + kind)
            node = next((node for node in candidates if isinstance(node, allowed)), None)
            if node is None and kind in {"function_definition","class_definition","assignment"}:
                matching=[node for end,node in by_start.get(item["start"],()) if end<=item["end"] and isinstance(node,allowed)]
                if len(matching)==1:node=matching[0]
            if node is None:
                raise ValueError(f"No CPython AST node for {kind} at {item['start']}..{item['end']}")
        row = {}
        for field in item["fields"]:
            if field == "syntax_error" and kind == "module":
                value = json.dumps(syntax_error, ensure_ascii=True, separators=(',', ':'))
            elif field == "dump_sha256":
                value = hashlib.sha256(ast.dump(node, include_attributes=False).encode("utf-8")).hexdigest()
            elif field == "literal_json":
                value = json.dumps(ast.literal_eval(node), sort_keys=True, separators=(",", ":"))
            elif field in {"unparse", "repr_unparse"}:
                value = ast.unparse(node)
                if field == "repr_unparse":
                    value = repr(value)
            elif field == "dump":
                value = ast.dump(node, include_attributes=False)
            elif field == "identifier":
                if isinstance(node, ast.Name):
                    value = node.id
                elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                    value = node.name
                elif isinstance(node, ast.Attribute):
                    value = node.attr
                else:
                    raise ValueError("Canonical identifier requires a named AST node")
            elif field in {"body_sha256", "signature_sha256"}:
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    raise ValueError("Function fingerprints require a function definition")
                if field == "body_sha256":
                    text = ast.dump(ast.Module(body=node.body, type_ignores=[]), include_attributes=False)
                else:
                    text = ast.dump(node.args, include_attributes=False)
                    text += "|" + (ast.dump(node.returns, include_attributes=False) if node.returns is not None else "None")
                    text += "|" + ast.dump(ast.Tuple(elts=node.type_params), include_attributes=False)
                value = hashlib.sha256(text.encode("utf-8")).hexdigest()
            else:
                raise ValueError("Unknown Python AST field: " + field)
            output_bytes += len(value.encode("utf-8"))
            if output_bytes > 64 * 1024 * 1024:
                raise ValueError("Python AST batch exceeds its output capacity")
            row[field] = value
        rows.append(row)
    output.append(rows)
json.dump({"version": [3, 13], "documents": output, "parses":parses, "module_walks":module_walks}, sys.stdout, ensure_ascii=True)
