"""Translate allowlist literals, first/last fact selection, and CLI report decisions."""
import json,re

SPECS=[('MAX_RENDER_STATIC_ALLOWLIST','ALLOWLISTED_STATIC_CLASSES','render() static allowlist','lint_grammar_render_methods.py'),
 ('MAX_RENDER_LEGACY_ALLOWLIST','LEGACY_ALLOWLISTED_CLASSES','render() legacy allowlist','lint_grammar_render_methods.py'),
 ('MAX_ESCAPE_HATCH_ALLOWLIST','ALLOWLISTED_CLASSES','escape hatch class allowlist','lint_sql_grammar_escape_hatches.py'),
 ('MAX_DATA_VALUE_PARAMS','DATA_VALUE_PARAMS','data value parameter allowlist','lint_sql_grammar_escape_hatches.py'),
 ('MAX_OPERATOR_PARAMS','OPERATOR_PARAMS','operator parameter allowlist','lint_sql_grammar_escape_hatches.py')]


def grammar_allowlist_burndown(source,target,header):
    from data_layer_rr_author import constants
    values=constants(source/'scripts/lint_grammar_allowlist_burndown.py')
    text=header('grammar_allowlist_burndown',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    text+='\n.decl gab_spec(i: number, key: symbol, variable: symbol, description: symbol, owner: symbol, maximum: number)\n'
    for i,(key,var,desc,owner) in enumerate(SPECS):text+='gab_spec('+', '.join([str(i),*[json.dumps(v) for v in [key,var,desc,owner]],str(values[key])])+').\n'
    return text+configuration()+inputs()+extraction()+outputs()+cli_report()


def configuration():
    return r'''
.decl gab_max(i: number, value: number)
gab_max(i, to_number(value)) :- gab_spec(i, key, _, _, _, _), option(key, value).
gab_max(i, value) :- gab_spec(i, key, _, _, _, value), !option(key, _).
.violation GABSpec(at: span, ordinal: number, key: symbol, variable: symbol, description: symbol, owner: symbol, maximum: number) code "GAB_SPEC" "spec"
GABSpec(@place("", 0), i, key, var, desc, owner, maximum) :- option("entryPoint", entry), (entry = "configuration" ; entry = "_allowlist_specs"), gab_spec(i, key, var, desc, owner, _), gab_max(i, maximum).
'''


def inputs():
    return r'''
.decl gab_path(d: number, path: symbol)
gab_path(d, path) :- doc(d, logical, _), doc_role(d, "primary"), option_item("documentPaths", i, logical), option_item("originalPaths", i, path).
gab_path(d, path) :- doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
.decl gab_scope_path(d: number, path: symbol)
gab_scope_path(d, path) :- doc(d, logical, _), option_item("documentPaths", i, logical), option_item("scopePaths", i, path).
gab_scope_path(d, path) :- doc(d, path, _), !option_item("documentPaths", _, path).
.decl gab_test(d: number)
gab_test(d) :- gab_scope_path(d, path), match_all("(?:\\A|/)(?:tests|test)(?:/|\\z)", path, _, _, _).
.decl gab_hidden(d: number)
gab_hidden(d) :- path_part(d, part), (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp").
.decl gab_selected(d: number)
gab_selected(d) :- option("entryPoint", entry), (entry = "facts" ; entry = "from_ast"), doc(d, path, _), doc_role(d, "primary"), suffix(".py", path), !gab_test(d).
gab_selected(d) :- !option("entryPoint", _), doc(d, path, _), doc_role(d, "primary"), prefix("scripts/", path), suffix(".py", path), !gab_test(d), !gab_hidden(d).
.decl gab_file(d: number)
gab_file(d) :- gab_selected(d), !syntax_error(d, _, _).
.violation GABSyntax(at: span, error: symbol) code "GAB_SYNTAX" "{error}"
GABSyntax(@place(path, 1), cat(message, " (", name, ", line ", to_string(l), ")")) :- option("raiseSyntaxErrors", "true"), gab_selected(d), syntax_error(d, l, message), gab_path(d, path), doc(d, logical, _), option_item("documentPaths", i, logical), option_item("originalNames", i, name).
'''


def extraction():
    return r'''
.decl gab_set_value(n: node, value: symbol)
gab_set_value(n, value) :- option("unimplementedEmptyDetector", "true"), kind(n, "set"), child(n, element, _), str_const(element, value).
.decl gab_frozen_value(n: node, value: symbol)
gab_frozen_value(n, value) :- gab_set_value(n, value).
.decl gab_assignment(d: number, a: node, target: node, name: symbol, ordinal: number, target_ordinal: number)
gab_assignment(d, a, target, @nfkc(name), ordinal, target_ordinal) :- gab_file(d), asgstmt(a, _), docof(a, d), assign_target(a, raw), unparen(raw, target), kind(target, "identifier"), text(target, name), visit_index(a, ordinal), visit_index(target, target_ordinal).
.decl gab_entry(d: number, a: node, target: node, name: symbol, ordinal: number, target_ordinal: number, value: symbol)
gab_entry(d, a, target, name, ordinal, target_ordinal, value) :- gab_assignment(d, a, target, name, ordinal, target_ordinal), assign_value(a, raw), unparen(raw, expr), gab_frozen_value(expr, value).
'''


def outputs():
    return r'''
.violation GABValue(at: span, value: symbol) code "GAB_VALUE" "{value}"
GABValue(@place("", 0), value) :- option("entryPoint", "_extract_set_values"), gab_set_value(0, value).
GABValue(@place("", 0), value) :- option("entryPoint", "_extract_frozenset_values"), gab_frozen_value(0, value).
GABValue(@place("", 0), value) :- option("entryPoint", "from_ast"), option("variableName", name), gab_entry(d, _, _, name, i, _, value), i = min j : { gab_entry(d, _, _, name, j, _, _) }.
.violation GABFact(at: span, variable_name: symbol, entry: symbol, count: number, ordinal: number, target_ordinal: number) code "GAB_FACT" "fact"
GABFact(@place(path, 1), name, value, count, i, ti) :- option("entryPoint", "facts"), gab_entry(d, a, target, name, i, ti, value), gab_path(d, path), count = count : { gab_entry(d, a, target, name, i, ti, _) }.
// The default reporter consumes the last nonempty fact. The CLI below has a separate owner-bound union contract.
.decl gab_last_path(name: symbol, path: symbol)
gab_last_path(name, path) :- gab_entry(_, _, _, name, _, _, _), path = max p : { gab_entry(d, _, _, name, _, _, _), gab_path(d, p) }.
.decl gab_last_ordinal(d: number, name: symbol, ordinal: number)
gab_last_ordinal(d, name, i) :- gab_last_path(name, path), gab_path(d, path), i = max j : { gab_entry(d, _, _, name, j, _, _) }.
.severity GABGrew hard-fail
.violation GABGrew(at: span, spec_ordinal: number, count: number, maximum: number, growth: number, description: symbol) code "allowlist_grew" "{description}: {count} > {maximum} (grew by {growth})"
GABGrew(@place(path, 1), order, count, maximum, count - maximum, desc) :- !option("entryPoint", _), gab_spec(order, _, name, desc, _, _), gab_max(order, maximum), gab_last_ordinal(d, name, i), gab_path(d, path), count = count : { gab_entry(d, _, _, name, i, _, _) }, count > maximum.
'''


def cli_report():
    return r'''
.decl gab_cli_entry(i: number, value: symbol)
gab_cli_entry(i, value) :- option("entryPoint", "report"), option_item("factVariables", row, name), option_item("factPaths", row, path), option_item("factEntries", row, value), gab_spec(i, _, name, _, owner, _), (path = owner ; suffix(cat("/", owner), path)).
.decl gab_cli_count(i: number, n: number, maximum: number)
gab_cli_count(i, n, maximum) :- option("entryPoint", "report"), gab_spec(i, _, _, _, _, _), gab_max(i, maximum), n = count : { gab_cli_entry(i, _) }.
.decl gab_cli_grew()
gab_cli_grew() :- gab_cli_count(_, n, maximum), n > maximum.
.decl gab_cli_suggest()
gab_cli_suggest() :- gab_cli_count(_, n, maximum), n < maximum.
.violation GABOutput(at: span, section: number, part: number, order: symbol, text: symbol) code "GAB_OUTPUT" "output"
GABOutput(@place("", 0), 0, 0, "", "Grammar Allowlist Burndown Report\n============================================================\n\n") :- option("entryPoint", "report").
GABOutput(@place("", 0), i + 1, 0, "", cat("  [FAIL] ", name, ": ", to_string(n), "/", to_string(maximum), " (EXCEEDED by ", to_string(n - maximum), ")\n")) :- gab_cli_count(i, n, maximum), gab_spec(i, _, _, name, _, _), n > maximum.
GABOutput(@place("", 0), i + 1, 0, "", cat("  [WARN] ", name, ": ", to_string(n), "/", to_string(maximum), " (at limit)\n")) :- gab_cli_count(i, n, maximum), gab_spec(i, _, _, name, _, _), n = maximum.
GABOutput(@place("", 0), i + 1, 0, "", cat("  [ OK ] ", name, ": ", to_string(n), "/", to_string(maximum), " (", to_string(maximum - n), " remaining)\n")) :- gab_cli_count(i, n, maximum), gab_spec(i, _, _, name, _, _), n < maximum.
GABOutput(@place("", 0), i + 1, 1, value, cat("         - ", value, "\n")) :- gab_cli_entry(i, value), option("verbose", "true").
GABOutput(@place("", 0), i + 1, 2, "", "\n") :- gab_cli_count(i, _, _), option("verbose", "true").
GABOutput(@place("", 0), 6, 0, "", cat("\nTotal allowlisted entries: ", to_string(total), "\n\n")) :- option("entryPoint", "report"), total = sum n : { gab_cli_count(_, n, _) }.
GABOutput(@place("", 0), 7, 0, "", "VIOLATIONS:\n") :- option("entryPoint", "report"), gab_cli_grew().
GABOutput(@place("", 0), 8, i, "", cat("  - ", name, ": ", to_string(n), " > ", to_string(maximum), " (grew by ", to_string(n - maximum), ")\n")) :- gab_cli_count(i, n, maximum), gab_spec(i, _, _, name, _, _), n > maximum.
GABOutput(@place("", 0), 9, 0, "", "\nAllowlists must not grow. Either:\n  1. Fix the violation using proper grammar classes\n  2. Reduce MAX_* constants after eliminating other entries\n\n") :- option("entryPoint", "report"), gab_cli_grew().
GABOutput(@place("", 0), 7, 0, "", "All allowlists within expected bounds.\n") :- option("entryPoint", "report"), !gab_cli_grew().
GABOutput(@place("", 0), 8, 0, "", "\nSuggested maximum reductions (after verifying removals are permanent):\n") :- option("entryPoint", "report"), !gab_cli_grew(), gab_cli_suggest().
GABOutput(@place("", 0), 9, i, "", cat("  ", name, ": reduce MAX from ", to_string(maximum), " to ", to_string(n), "\n")) :- gab_cli_count(i, n, maximum), gab_spec(i, _, _, name, _, _), !gab_cli_grew(), n < maximum.
.violation GABExit(at: span, status: number) code "GAB_EXIT" "exit"
GABExit(@place("", 0), 1) :- option("entryPoint", "report"), gab_cli_grew().
GABExit(@place("", 0), 0) :- option("entryPoint", "report"), !gab_cli_grew().
'''
