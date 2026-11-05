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
KINDS.update({
    'import_statement':(ast.Import,), 'future_import_statement':(ast.ImportFrom,),
    'aliased_import':(ast.alias,), 'dotted_name':(ast.alias,),
    'expression_statement':(ast.Expr,), 'augmented_assignment':(ast.AugAssign,),
    'return_statement':(ast.Return,), 'if_statement':(ast.If,), 'elif_clause':(ast.If,),
    'for_statement':(ast.For,ast.AsyncFor), 'while_statement':(ast.While,),
    'with_statement':(ast.With,ast.AsyncWith), 'try_statement':(ast.Try,ast.TryStar),
    'except_clause':(ast.ExceptHandler,), 'raise_statement':(ast.Raise,),
    'assert_statement':(ast.Assert,), 'pass_statement':(ast.Pass,), 'break_statement':(ast.Break,),
    'continue_statement':(ast.Continue,), 'delete_statement':(ast.Delete,),
    'keyword_argument':(ast.keyword,), 'dictionary_splat':(ast.keyword,), 'lambda':(ast.Lambda,), 'await':(ast.Await,),
    'boolean_operator':(ast.BoolOp,), 'binary_operator':(ast.BinOp,), 'union_type':(ast.BinOp,),
    'generic_type':(ast.Subscript,), 'not_operator':(ast.UnaryOp,), 'unary_operator':(ast.UnaryOp,),
    'comparison_operator':(ast.Compare,), 'conditional_expression':(ast.IfExp,),
    'list_comprehension':(ast.ListComp,), 'dictionary_comprehension':(ast.DictComp,),
    'set_comprehension':(ast.SetComp,), 'generator_expression':(ast.GeneratorExp,),
    'named_expression':(ast.NamedExpr,), 'list_splat':(ast.Starred,),
    'tuple_pattern':(ast.Tuple,), 'list_pattern':(ast.List,), 'pattern_list':(ast.Tuple,),
    'set':(ast.Set,), 'ellipsis':(ast.Constant,),
    'default_parameter':(ast.arg,), 'typed_parameter':(ast.arg,), 'typed_default_parameter':(ast.arg,),
})
output = []
output_bytes = 0
module_walks = 0
parses = 0
for document in request["documents"]:
    source = document["source"]
    def text_only(item):
        return set(item['fields']) <= set(TEXT_FIELDS) or item['fields']==['column'] or item['kind'] in {'identifier','dotted_name','relative_import'} and item['fields']==['identifier']
    tree = None
    syntax_error = None
    if not all(text_only(item) for item in document['requests']):
        parses += 1
        try:
            tree = ast.parse(source)
        except SyntaxError as error:
            # A relation over an invalid module has no syntax rows. Explicit
            # syntax_error requests still expose the error in the same batch.
            # Scalar AST projections keep their strict failure contract.
            if not all(text_only(item) or item['kind'] == 'module' and item['fields'] in (
                           ['syntax_error'], ['ast_nodes'], ['ast_annotations'])
                       for item in document['requests']):
                raise
            syntax_error = {name: getattr(error, name) for name in (
                'msg', 'lineno', 'offset', 'text', 'end_lineno', 'end_offset')}
    source_bytes=source.encode("utf-8")
    offsets = [0, *(match.end() for match in re.finditer(rb"\r\n?|\n",source_bytes))]
    by_span = {}
    by_start = {}
    structural=any(any(field in STRUCTURE_FIELDS or field in {'annotation_names','ast_nodes','ast_annotations'} for field in item['fields']) for item in document['requests'])
    details={}
    needed_types=tuple({kind for item in document["requests"] if item["kind"]!="module" and not text_only(item) for kind in KINDS.get(item["kind"],())})
    module_walks += bool(tree is not None and (needed_types or structural))
    if structural and tree is not None:
        requested={field for item in document['requests'] for field in (item.get('columns',STRUCTURE_FIELDS) if item['fields']==['ast_nodes'] else item['fields'])}
        details,all_nodes=structure_index(tree,offsets,len(source_bytes),breadth='ast_breadth_order' in requested,annotations=bool({'ast_annotation','ast_annotation_repr'}&requested))
    else:all_nodes=ast.walk(tree) if tree is not None and needed_types else ()
    for node in all_nodes:
        if (not structural and not isinstance(node,needed_types)) or not hasattr(node,"lineno"):continue
        span=(offsets[node.lineno-1]+node.col_offset,offsets[node.end_lineno-1]+node.end_col_offset)
        by_span.setdefault(span,[]).append(node)
        by_start.setdefault(span[0],[]).append((span[1],node))
    source_lines=source.splitlines()
    rows = []
    for item in document["requests"]:
        kind = item["kind"]
        if item['fields'] in (['ast_nodes'],['ast_annotations']):
            if kind!='module':raise ValueError('Canonical relation expansion requires a module root')
            if item['fields']==['ast_nodes']:
                # Operators and contexts are shared singleton objects. They are
                # not located syntax nodes; each located node keeps its own edge.
                selected=[node for node in all_nodes if hasattr(node,'lineno') or bool(node._fields)]
                fields=list(dict.fromkeys([*item.get('columns',STRUCTURE_FIELDS),'ast_start','ast_stop']))
                if any(field not in STRUCTURE_FIELDS for field in fields):raise ValueError('Unknown canonical expansion column')
                expanded={field:[details[id(node)][field] for node in selected] for field in fields}
            else:
                roots=[node for node in all_nodes if details[id(node)]['ast_field'] in {'annotation','returns'}]
                selected={id(node):node for root in roots for node in ast.walk(root) if node is root or isinstance(node,ast.expr) and details[id(node)]['ast_field'] in {'value','slice'} and details[id(node)]['ast_parent_kind']=='Subscript'}
                pairs=[(details[id(node)]['ast_preorder'],name) for node in selected.values() for name in annotation_names(node)]
                expanded=dict(annotation=[p[0] for p in pairs],name=[p[1] for p in pairs])
            value=json.dumps(expanded,ensure_ascii=True,separators=(',',':'))
            output_bytes+=len(value)
            if output_bytes>64*1024*1024:raise ValueError('Python AST batch exceeds its output capacity')
            rows.append({item['fields'][0]:value});continue
        if set(item['fields']) <= set(TEXT_FIELDS):
            line=bisect.bisect_right(offsets,item['start'])-1
            row={field:str(len(source_lines)) if field=='source_lines' else source_lines[line] if field=='source_line' and line<len(source_lines) else source_lines[line-1] if field=='previous_line' and 0<line<=len(source_lines) else '' for field in item['fields']}
            output_bytes+=sum(len(value.encode('utf-8')) for value in row.values())
            if output_bytes>64*1024*1024:raise ValueError('Python AST batch exceeds its output capacity')
            rows.append(row);continue
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
            metadata=any(field in STRUCTURE_FIELDS for field in item['fields'])
            allowed = (ast.Name,ast.arg) if metadata and kind=='identifier' else KINDS.get(kind)
            if allowed is None and not metadata:
                raise ValueError("Unsupported canonical AST root kind: " + kind)
            node = next((node for node in candidates if allowed is not None and isinstance(node, allowed)), None)
            if node is None and kind in {"function_definition","class_definition","assignment"}:
                matching=[node for end,node in by_start.get(item["start"],()) if end<=item["end"] and isinstance(node,allowed)]
                if len(matching)==1:node=matching[0]
            if node is None and metadata and allowed is not None:
                matches=[candidate for _,candidate in by_start.get(item['start'],()) if isinstance(candidate,allowed)]
                if not matches and ast.arg in allowed:
                    matches=[candidate for (start,end),parts in by_span.items() if item['start']<=start<=item['start']+2 and end<=item['end'] for candidate in parts if isinstance(candidate,ast.arg)]
                if len(matches)==1:node=matches[0]
            if node is None and (not metadata or not set(item['fields'])<=set(STRUCTURE_FIELDS)):
                raise ValueError(f"No CPython AST node for {kind} at {item['start']}..{item['end']}")
        row = {}
        for field in item["fields"]:
            if field in STRUCTURE_FIELDS:
                value=details[id(node)][field] if node is not None else ''
            elif field=='annotation_names':
                value=json.dumps(annotation_names(node),ensure_ascii=True)
            elif field == "syntax_error" and kind == "module":
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
