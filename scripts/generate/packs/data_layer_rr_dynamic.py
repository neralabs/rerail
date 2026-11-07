"""Translate the source scanner's state machine, including its deliberate textual heuristics."""
import json
from data_layer_rr_text import classes, module, search


def dynamic_module_loading(source, target, header):
    original = module(source, 'dynamic_module_loading')
    _, space, _ = classes()
    q = json.dumps
    text = header('dynamic_module_loading', target).replace('#include "python/ast.rr"\n', '#include "data-layer/signing.rr"\n')
    trim = r'\A' + space + r'+|' + space + r'+\z'
    left = r'\A' + space + '+'
    text += '.decl dml_line(d: number, l: number, t: symbol, clean: symbol, indent: number)\n'
    text += f'dml_line(d, l, t, @replace_all(t, {q(trim)}, ""), strlen(t) - strlen(@replace_all(t, {q(left)}, ""))) :- split_line(d, l, t).\n'
    text += f'dml_line(0, i + 1, t, @replace_all(t, {q(trim)}, ""), strlen(t) - strlen(@replace_all(t, {q(left)}, ""))) :- option_item("helperLines", i, t).\n'
    text += '.decl dml_quotes(d: number, l: number, double: number, single: number)\n'
    text += f'dml_quotes(d, l, (strlen(t) - strlen(@replace_all(t, {q(chr(34)*3)}, ""))) / 3, (strlen(t) - strlen(@replace_all(t, {q(chr(39)*3)}, ""))) / 3) :- split_line(d, l, t).\n'
    text += '''
// This scanner distinguishes quote delimiters and checks double quotes first.
.decl dml_next(d: number, l: number, next: number)
dml_next(d, l, next) :- split_line(d, l, _), split_line(d, next, _), next = l + 1.
.decl dml_state(d: number, l: number, quote: symbol)
dml_state(d, 1, "") :- split_line(d, 1, _).
dml_state(d, next, "double") :- dml_state(d, l, ""), dml_quotes(d, l, 1, _), dml_next(d, l, next).
dml_state(d, next, "single") :- dml_state(d, l, ""), dml_quotes(d, l, 0, 1), dml_next(d, l, next).
dml_state(d, next, "") :- dml_state(d, l, ""), dml_quotes(d, l, double, single),
    (double >= 2 ; double = 0, single != 1), dml_next(d, l, next).
dml_state(d, next, "") :- dml_state(d, l, "double"), dml_quotes(d, l, double, _), double > 0, dml_next(d, l, next).
dml_state(d, next, "double") :- dml_state(d, l, "double"), dml_quotes(d, l, 0, _), dml_next(d, l, next).
dml_state(d, next, "") :- dml_state(d, l, "single"), dml_quotes(d, l, _, single), single > 0, dml_next(d, l, next).
dml_state(d, next, "single") :- dml_state(d, l, "single"), dml_quotes(d, l, _, 0), dml_next(d, l, next).
.decl dml_trigger(d: number)
'''
    for marker in original._FAST_SUBSTRINGS:
        text += f'dml_trigger(d) :- source(d, sp), text(sp, t), contains({q(marker)}, t).\n'
    text += '.decl dml_allowed(p: symbol)\ndml_allowed(p) :- option_item("allowlistedFiles", _, p).\n'
    text += '.decl dml_candidate(d: number, l: number, ordinal: number, label: symbol)\n'
    patterns = [*original._PATTERNS, *original._IMPORT_MODULE_PATTERNS]
    for i, (pattern, label) in enumerate(patterns):
        text += f'dml_candidate(d, l, {i}, {q(label)}) :- dml_trigger(d), split_line(d, l, t), match({q(search(pattern))}, t).\n'
        text += f'\n.violation DynamicPattern{i}(at: span) code "DML_PATTERN" "matched"\n'
        text += f'DynamicPattern{i}(@place("input.py", 1)) :- option("entryPoint", "pattern_search"), option("patternOrdinal", "{i}"), option("patternText", t), match({q(search(pattern))}, t).\n'
    text += '''
.decl dml_primary(d: number, l: number)
dml_primary(d, l) :- dml_candidate(d, l, ordinal, _), ordinal < 3.
.decl dml_demand(d: number, l: number)
dml_demand(d, l) :- dml_candidate(d, l, _, _).
dml_demand(d, l) :- option("entryPoint", "_is_in_type_checking_block"), option("lineIndex", i),
    dml_line(d, l, _, _, _), l = to_number(i) + 1.
.decl dml_meaningful(d: number, l: number, clean: symbol, indent: number)
dml_meaningful(d, l, clean, indent) :- dml_line(d, l, _, clean, indent), clean != "", !prefix("#", clean).
.decl dml_cut(d: number, before: number, at: number)
dml_cut(d, before, at) :- dml_demand(d, at), dml_meaningful(d, before, _, _), before < at,
    dml_meaningful(d, boundary, _, 0), boundary > before, boundary < at.
.decl dml_type_checking(d: number, l: number)
dml_type_checking(d, l) :- dml_demand(d, l), dml_line(d, l, _, _, indent),
    dml_meaningful(d, before, clean, previous_indent), before < l, previous_indent < indent,
    contains("TYPE_CHECKING", clean), !dml_cut(d, before, l).
.violation DynamicTypeCheckingProbe(at: span) code "DML_TYPE_CHECKING" "inside TYPE_CHECKING"
DynamicTypeCheckingProbe(@place("input.py", l)) :- option("entryPoint", "_is_in_type_checking_block"),
    dml_type_checking(_, l).
.violation DynamicLoading(at: span, ordinal: number, snippet: symbol, label: symbol) code "DML001" tag {ordinal} "{label}"
DynamicLoading(@place(p, l), ordinal, clean, label) :- doc(d, p, _), !dml_allowed(p),
    !option("entryPoint", "_is_in_type_checking_block"), dml_candidate(d, l, ordinal, label),
    dml_line(d, l, _, clean, _), !prefix("#", clean), dml_state(d, l, ""), dml_quotes(d, l, 0, 0),
    !dml_type_checking(d, l), (ordinal < 3 ; ordinal >= 3, !dml_primary(d, l)).
.decl dml_signature(s: symbol)
dml_signature(s) :- option("signature", s).
'''
    text += f'dml_signature({q(original.ALLOWLIST_SIGNATURE)}) :- !option("signature", _).\n'
    text += f'allow_sig("dynamic_module_loading", {q(original.LINTER_NAME)}, s) :- dml_signature(s).\n'
    text += '''allow_entry("dynamic_module_loading", p) :- dml_allowed(p).
.violation DynamicSignature(at: span) code "DML000" "dynamic module loading allowlist signature verification failed"
DynamicSignature(@place("scripts/lint_dynamic_module_loading.py", 1)) :- !option("entryPoint", _),
    allow_status("dynamic_module_loading", status), status != "valid".
'''
    return text
