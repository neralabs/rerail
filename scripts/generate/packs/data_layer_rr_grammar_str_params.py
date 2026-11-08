"""Translate grammar string parameters, constructor prepasses, and ordered render events."""
import json
from data_layer_rr_text import module


def grammar_str_params(source,target,header):
    original=module(source,'grammar_str_params');q=json.dumps
    text=header('grammar_str_params',target)+'#include "python/print.rr"\n'
    text+='.decl gsp_default_class(name: symbol)\n'+''.join(f'gsp_default_class({q(name)}).\n' for name in sorted(original.ALLOWLISTED_STR_CLASSES))
    text+='.decl gsp_default_param(cls: symbol, name: symbol)\n'+''.join(f'gsp_default_param({q(cls)}, {q(name)}).\n' for cls,name in sorted(original.ALLOWLISTED_STR_PARAMS))
    text+='.decl gsp_baseline(order: number, code: symbol, count: number)\n'+''.join(f'gsp_baseline({i}, {q(code)}, {count}).\n' for i,(code,count) in enumerate(original.BASELINE_COUNTS.items()))
    hint='\nMechanical cleanup:\n  uv run python scripts/fix_grammar_linter_violations.py --write\n  uv run python scripts/codemod_hoist_render_raw_strings.py impl/common/l2100_sql_grammar\n'
    text+='.decl gsp_hint(value: symbol)\ngsp_hint('+q(hint)+').\n'
    return text+configuration()+detector()+outputs()


def configuration():
    return r'''
.violation GSPClass(at: span, name: symbol) code "GSP_CLASS" "{name}"
GSPClass(@place("", 0), name) :- option("entryPoint", "configuration"), gsp_default_class(name).
.violation GSPParam(at: span, cls: symbol, name: symbol) code "GSP_PARAM" "{name}"
GSPParam(@place("", 0), cls, name) :- option("entryPoint", "configuration"), gsp_default_param(cls, name).
.violation GSPBaseline(at: span, ordinal: number, code: symbol, count: number) code "GSP_BASELINE" "{code}"
GSPBaseline(@place("", 0), i, code, count) :- option("entryPoint", "configuration"), gsp_baseline(i, code, count).
.violation GSPText(at: span, value: symbol) code "GSP_TEXT" "{value}"
GSPText(@place("", 0), value) :- option("entryPoint", "print_autofix_hint"), gsp_hint(value).
.decl gsp_allow_class(name: symbol)
gsp_allow_class(name) :- option("entryPoint", _), option_item("classes", _, name).
gsp_allow_class(name) :- !option("entryPoint", _), gsp_default_class(name).
.decl gsp_allow_param(cls: symbol, name: symbol)
gsp_allow_param(cls, name) :- option("entryPoint", _), option_item("paramClasses", i, cls), option_item("paramNames", i, name).
gsp_allow_param(cls, name) :- !option("entryPoint", _), gsp_default_param(cls, name).
.decl gsp_trigger(text: symbol)
gsp_trigger("__init__"). gsp_trigger("render("). gsp_trigger("str"). gsp_trigger("Sql").
.violation GSPRead(at: span) code "GSP_READ" "read"
GSPRead(@place("", 0)) :- option("entryPoint", "source_gate"), option("providedSource", source), gsp_trigger(t), contains(t, source).
'''


def detector():
    return r'''
.decl gsp_result(path: symbol, line: number, cls: symbol, name: symbol, annotation: symbol, code: symbol, detail: symbol, ordinal: number, first: number, second: number)
'''


def outputs():
    text=r'''
.violation GSPFact(at: span, class_name: symbol, param_name: symbol, annotation: symbol, violation_type: symbol, detail: symbol, ordinal: number, first: number, second: number) code "GSP_FACT" "{detail}"
GSPFact(@place(path, line), cls, name, annotation, code, detail, i, a, b) :- option("entryPoint", _), gsp_result(path, line, cls, name, annotation, code, detail, i, a, b).
.decl gsp_exceeded(code: symbol)
gsp_exceeded(code) :- gsp_baseline(_, code, baseline), n = count : { gsp_result(_, _, _, _, _, code, _, _, _, _) }, n > baseline.
'''
    for name,code in [('GSPStringParam','STR_PARAM'),('GSPStringCall','STR_CALL_IN_RENDER'),('GSPStringConcat','STR_CONCAT_IN_RENDER')]:
        text+=f'.severity {name} error\n.violation {name}(at: span, cls: symbol, name: symbol, detail: symbol, ordinal: number, first: number, second: number) code {json.dumps(code)} "{{cls}}.{{name}}: {{detail}}"\n'
        text+=f'{name}(@place(path, line), cls, name, detail, i, a, b) :- !option("entryPoint", _), gsp_exceeded({json.dumps(code)}), gsp_result(path, line, cls, name, _, {json.dumps(code)}, detail, i, a, b).\n'
    return text
