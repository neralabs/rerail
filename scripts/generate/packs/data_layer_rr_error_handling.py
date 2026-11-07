"""Port exception-body adjacency and statement-level transaction commits."""
import json
from data_layer_rr_text import module


def error_handling(source, target, header):
    original = module(source, 'error_handling')
    text = header('error_handling', target).replace(', "**/test_*.py"', '')
    text += '.decl eh_dir(which: symbol, prefix: symbol)\n'
    for kind, values in [('log', original._LOG_RERAISE_DIRS), ('txn', original._TXN_COMMIT_DIRS)]:
        for value in values:
            text += f'eh_dir({json.dumps(kind)}, {json.dumps(value)}).\n'
    return text + r'''
.decl eh_hidden(d: number)
eh_hidden(d) :- path_part(d, p), (p = "tests" ; p = "test").
eh_hidden(d) :- doc(d, _, _), option_item("pathParts", _, p), (p = "tests" ; p = "test").
.decl eh_framework_hidden(d: number)
eh_framework_hidden(d) :- path_part(d, p),
    (p = ".git" ; p = ".venv" ; p = "venv" ; p = "__pycache__" ; p = "tmp").
.decl eh_path(d: number, path: symbol)
eh_path(d, path) :- doc(d, _, _), option("entryPoint", "scan_file_content"), option("relativePath", path),
    option("filename", filename), suffix(".py", filename), !eh_hidden(d),
    !option("outsideRoot", "true"), !option("relativeRoot", "true"), option("providedSource", provided),
    (contains("logger.warning", provided) ; contains("logger.error", provided) ; contains("txn.commit", provided)).
eh_path(d, path) :- doc(d, path, _), !option("entryPoint", _), !eh_hidden(d), !eh_framework_hidden(d),
    file_name(d, filename), suffix(".py", filename), eh_dir(_, dir), prefix(cat(dir, "/"), path).
.decl eh_active(d: number, path: symbol)
eh_active(d, path) :- eh_path(d, path), !syntax_error(d, _, _), source(d, sp), text(sp, source),
    (contains("logger.warning", source) ; contains("logger.error", source) ; contains("txn.commit", source)).
.decl eh_expr_call(s: node, c: node)
eh_expr_call(s, c) :- kind(s, "expression_statement"), child(s, e, 0), unparen(e, c), kind(c, "call").
.decl eh_method(c: node, receiver: symbol, method: symbol)
eh_method(c, @nfkc(receiver), @nfkc(method)) :- kind(c, "call"), field(c, "function", f), unparen(f, fn),
    kind(fn, "attribute"), field(fn, "object", raw), unparen(raw, obj), ast_name(obj, receiver),
    field(fn, "attribute", name), text(name, method).
.decl eh_logger(s: node)
eh_logger(s) :- eh_expr_call(s, c), eh_method(c, "logger", name), (name = "warning" ; name = "error").
.decl eh_raise(r: node)
eh_raise(r) :- kind(r, "raise_statement"), !child(r, _, _).
.violation LogReraise(at: node, ordinal: number, witness: number) code "EH001"
    "log-then-reraise: logger call followed by bare raise"
LogReraise(r, rank, i) :- kind(e, "except_clause"), child(e, b, _), kind(b, "block"),
    body_stmt(b, s, i), body_stmt(b, r, i + 1), eh_logger(s), eh_raise(r), visit_index(e, rank),
    docof(e, d), eh_active(d, path), eh_dir("log", prefix), prefix(prefix, path).
.decl eh_function(c: node, name: symbol)
eh_function(c, @nfkc(name)) :- kind(c, "call"), def_frame(c, f), def_name(f, name).
eh_function(c, "<module>") :- kind(c, "call"), !def_frame(c, _).
.violation AdminCommit(at: node, ordinal: number, witness: number, function: symbol) code "EH001"
    "explicit txn.commit() in function '{function}'"
AdminCommit(s, rank, 0, function) :- eh_expr_call(s, c), eh_method(c, "txn", "commit"),
    eh_function(c, function), visit_index(c, rank), docof(c, d), eh_active(d, path),
    eh_dir("txn", prefix), prefix(prefix, path).
'''
