"""Translate callable grammar constructors onto shared native syntax relations."""
import json,re


def grammar_callable_constructors(source,target,header):
    text=header('grammar_callable_constructors',target)+'#include "project.rr"\n'
    text=re.sub(r'^\.scope include .*$', '.scope include "impl/common/l2100_sql_grammar/**/*.py", "scripts/lint_grammar_callable_constructors.py"',text,flags=re.M)
    return text+configuration()+detector()+outputs()


def configuration():
    return r'''
.violation GCCFormatted(at: span, key: symbol, formatted: symbol) code "GCC_FORMAT" "{formatted}"
GCCFormatted(@place("", 0), cat(path, ":", line, ":", code, ":", detail), cat(path, ":", line, ": ", code, ": ", detail)) :-
 option("entryPoint", "format"), option("formatPath", path), option("formatLine", line), option("formatCode", code), option("formatDetail", detail).
'''


def detector():
    return r'''
.decl gcc_result(d: number, line: number, code: symbol, detail: symbol, origin: node, tag: symbol)
.decl gcc_path(d: number, path: symbol)
gcc_path(d, original) :- doc(d, logical, _), doc_role(d, "primary"), option_item("documentPaths", i, logical), option_item("originalPaths", i, original).
gcc_path(d, path) :- doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
'''


def outputs():
    text=r'''
.violation GCCFact(at: span, violation_type: symbol, detail: symbol, key: symbol, formatted: symbol, origin: node, tag: symbol) code "GCC_FACT" "{detail}"
GCCFact(@place(path, line), code, detail, cat(path, ":", to_string(line), ":", code, ":", detail), cat(path, ":", to_string(line), ": ", code, ": ", detail), n, tag) :-
 option("entryPoint", _), gcc_result(d, line, code, detail, n, tag), gcc_path(d, path).
'''
    for i,code in enumerate(['CALLABLE_INIT_PARAM','CALLABLE_FIELD','CONSTRUCTOR_FIELD_CALLED_IN_RENDER','UNTYPED_CALLED_CONSTRUCTOR_FIELD','ANY_CALLED_CONSTRUCTOR_FIELD','CALLABLE_CONSTRUCTOR_ARG','SIMPLE_NAMESPACE_RENDER_HOOK','DYNAMIC_RENDER_ASSIGNMENT']):
        name='GCCFinding'+str(i)
        text+=f'.severity {name} error\n.violation {name}(at: span, origin: node, tag: symbol, detail: symbol) code {json.dumps(code)} "{{detail}}"\n'
        text+=f'{name}(@place(path, line), n, tag, detail) :- !option("entryPoint", _), gcc_result(d, line, {json.dumps(code)}, detail, n, tag), gcc_path(d, path).\n'
    return text
