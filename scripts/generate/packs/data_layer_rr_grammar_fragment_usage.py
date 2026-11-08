"""Translate fragment usage over the modern engine's Python syntax relations."""
import re


def grammar_fragment_usage(source,target,header):
    text=header('grammar_fragment_usage',target)+'#include "project.rr"\n'
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    return text+inputs()+detector()+outputs()


def inputs():
    return r'''
.decl gfu_path(d: number, path: symbol)
gfu_path(d, path) :- doc(d, logical, _), doc_role(d, "primary"), option_item("documentPaths", i, logical), option_item("originalPaths", i, path).
gfu_path(d, path) :- doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
.decl gfu_hidden(d: number)
gfu_hidden(d) :- path_part(d, part), (part = "test" ; part = "tests" ; part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp").
.decl gfu_selected(d: number)
gfu_selected(d) :- option("entryPoint", "files"), doc(d, path, _), doc_role(d, "primary"), suffix(".py", path).
gfu_selected(d) :- !option("entryPoint", _), doc(d, path, _), doc_role(d, "primary"), suffix(".py", path), !gfu_hidden(d), (prefix("_internal/", path) ; prefix("api/", path) ; prefix("data_layer/", path) ; prefix("impl/", path) ; match_all("\\Al[0-9][^/]*/", path, _, _, _)).
.decl gfu_file(d: number)
gfu_file(d) :- gfu_selected(d), !syntax_error(d, _, _).
.violation GFUFormat(at: span, value: symbol) code "GFU_FORMAT" "{value}"
GFUFormat(@place("", 0), cat(path, ":", line, ": ", cls, ": ", detail)) :- option("entryPoint", "format"), option("formatPath", path), option("formatLine", line), option("formatClass", cls), option("formatMessage", detail).
.violation GFUHint(at: span, value: symbol) code "GFU_HINT" "{value}"
GFUHint(@place("", 0), "\nMechanical cleanup:\n  uv run python scripts/fix_grammar_linter_violations.py --write\n  uv run python scripts/rollup_sql_fragments.py --write\n") :- option("entryPoint", "print_autofix_hint").
'''


def detector():
    return r'''
.decl gfu_result(d: number, line: number, cls: symbol, detail: symbol, origin: node, tag: symbol)
gfu_result(d, 0, "", "", n, "") :- gfu_file(d), kind(n, "module"), docof(n, d), option("unimplemented-empty-detector", "true").
'''


def outputs():
    return r'''
.violation GFUFact(at: span, class_name: symbol, detail: symbol, origin: node, tag: symbol) code "GFU_FACT" "{detail}"
GFUFact(@place(path, l), cls, detail, n, tag) :- option("entryPoint", "files"), gfu_result(d, l, cls, detail, n, tag), gfu_path(d, path).
.severity GFUFinding hard-fail
.violation GFUFinding(at: span, class_name: symbol, detail: symbol, origin: node, tag: symbol) code "sqlfragment_usage" "{class_name}: {detail}"
GFUFinding(@place(path, l), cls, detail, n, tag) :- !option("entryPoint", _), gfu_result(d, l, cls, detail, n, tag), gfu_path(d, path).
.decl gfu_parse(d: number, detail: symbol)
gfu_parse(d, cat("Cannot inspect Python syntax: SyntaxError: ", message, " (", name, ", line ", to_string(l), ")")) :- gfu_selected(d), syntax_error(d, l, message), file_name(d, logical_name), !option_item("documentPaths", _, _), name = logical_name.
gfu_parse(d, cat("Cannot inspect Python syntax: SyntaxError: ", message, " (", name, ", line ", to_string(l), ")")) :- gfu_selected(d), syntax_error(d, l, message), doc(d, logical, _), option_item("documentPaths", i, logical), option_item("originalNames", i, name).
.violation GFUParseFact(at: span, class_name: symbol, detail: symbol) code "GFU_FACT" "{detail}"
GFUParseFact(@place(path, 1), "<parse>", detail) :- option("entryPoint", "files"), gfu_parse(d, detail), gfu_path(d, path).
.severity GFUParse hard-fail
.violation GFUParse(at: span) code "python_parse_error" "<parse>: {detail}"
GFUParse(@place(path, 1)) :- !option("entryPoint", _), gfu_parse(d, detail), gfu_path(d, path).
'''.replace('GFUParse(at: span) code','GFUParse(at: span, detail: symbol) code').replace('GFUParse(@place(path, 1))','GFUParse(@place(path, 1), detail)')
