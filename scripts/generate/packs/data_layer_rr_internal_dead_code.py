"""Translate dead-code survey decisions over the shared native graph contract."""
import ast,json,os,re,subprocess,sys
from data_layer_rr_text import classes


def internal_dead_code(source,target,header):
    text=header('internal_dead_code',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    module=ast.parse((source/'scripts/lint_internal_dead_code.py').read_text())
    regex=next(ast.literal_eval(n.value.args[0]) for n in module.body if isinstance(n,ast.Assign) and isinstance(n.targets[0],ast.Name) and n.targets[0].id=='_LITERAL_DYNAMIC_IMPORT_ATTR_RE')
    word,space,_=classes()
    head=r'(?:\bimport_module|\bload_python_module|\bimportlib\.import_module)'
    if not regex.startswith(head):raise ValueError('Review the changed dynamic-import pattern')
    regex=r'(?:\A|[^'+word[1:]+r')(?:import_module|load_python_module|importlib\.import_module)'+regex[len(head):]
    regex=regex.replace(r'\s',space).replace('(?P<','(?<')
    execute=next(n.value for n in module.body if isinstance(n,ast.AnnAssign) and isinstance(n.target,ast.Name) and n.target.id=='VALID_EXECUTE_CATEGORIES')
    code='import json; print(json.dumps(list('+ast.unparse(execute)+')))'
    order=json.loads(subprocess.check_output([sys.executable,'-c',code],env=dict(os.environ,PYTHONHASHSEED='0'),text=True))
    text+='#include "data-layer/survey-graph.rr"\n'+empty_outputs()
    text+='\n.decl idc_execute_category(i: number, category: symbol)\n'
    text+=''.join(f'idc_execute_category({i}, {json.dumps(name)}).\n' for i,name in enumerate(order))
    text+='\nIDCValue(@place("", 0), cat(module, ".", attr)) :- option("entryPoint", "_collect_literal_imported_fqnames"), option("source", source), match_all('+json.dumps(regex)+', source).\n'
    return text+helper_relations()+analysis_relations()


def empty_outputs():
    return r'''
.violation IDCValue(at: span, value: symbol) code "IDC_VALUE" "{value}"
IDCValue(@place("", 0), "") :- option("entryPoint", _), 1 = 0.
.violation IDCGroup(at: span, category: symbol, ordinal: number) code "IDC_GROUP" "category"
IDCGroup(@place("", 0), "", 0) :- option("entryPoint", _), 1 = 0.
.violation IDCItem(at: span, category: symbol, sid: symbol, ordinal: number) code "IDC_ITEM" "item"
IDCItem(@place("", 0), "", "", 0) :- option("entryPoint", _), 1 = 0.
.violation IDCTarget(at: span, path: symbol, sid: symbol, ordinal: number) code "IDC_TARGET" "target"
IDCTarget(@place("", 0), "", "", 0) :- option("entryPoint", _), 1 = 0.
.violation IDCKeepResults(at: span) code "IDC_KEEP_RESULTS" "keep input"
IDCKeepResults(@place("", 0)) :- option("entryPoint", _), 1 = 0.
.violation IDCSourceGraphRequired(at: span) code "IDC_SOURCE_GRAPH_REQUIRED" "Native source graph production is not yet implemented for this draft"
IDCSourceGraphRequired(@place("", 0)) :- !option("entryPoint", _).
'''


def helper_relations():
    return r'''
.decl idc_import_root(sid: symbol)
idc_import_root(sid) :- option_item("importedFqNames", _, fq), sg_resolved(fq, sid).
.decl idc_public(c: symbol, sid: symbol)
idc_public(c, sid) :- option_item("providedPublicClasses", i, c), option_item("providedPublicSymbols", i, sid).
idc_public(c, sid) :- cm_public(c, _, sid).
.decl idc_dunder(c: symbol, sid: symbol)
idc_dunder(c, sid) :- option_item("providedDunderClasses", i, c), option_item("providedDunderSymbols", i, sid).
idc_dunder(c, sid) :- cm_dunder(c, _, sid).
.decl idc_missing_import(i: number, sid: symbol)
idc_missing_import(i, missing) :- option_item("importedFqNames", i, fq), sg_resolved(fq, c), sg_class(c), cm_first_missing(c, missing).
.violation IDCGraphMethodMissing(at: span, sid: symbol) code "IDC_GRAPH_METHOD_MISSING" "Missing class method symbol {sid}"
IDCGraphMethodMissing(@place("", 0), sid) :- option("entryPoint", "_resolve_dynamic_import_roots"), idc_missing_import(i, sid), i = min j : { idc_missing_import(j, _) }.
IDCValue(@place("", 0), sid) :- option("entryPoint", "_resolve_dynamic_import_roots"), idc_import_root(sid).
IDCValue(@place("", 0), method) :- option("entryPoint", "_resolve_dynamic_import_roots"), idc_import_root(sid), sg_class(sid), (idc_public(sid, method) ; idc_dunder(sid, method)).
.decl idc_excluded(sid: symbol)
idc_excluded(sid) :- sg_symbol(_, sid, _, _, _, raw, _, _, _, _), path = @replace_all(raw, "\\\\", "/"), option_item("excludedPaths", _, prefix), (path = @replace_all(prefix, "/\\z", "") ; prefix(prefix, path)).
.decl idc_supplied(category: symbol, sid: symbol, ordinal: number)
idc_supplied(category, sid, to_number(i)) :- option_item("resultRowCategories", row, category), option_item("resultRowSymbols", row, sid), option_item("resultRowOrdinals", row, i).
IDCKeepResults(@place("", 0)) :- option("entryPoint", "_filter_results_by_excluded_paths"), !option_item("excludedPaths", _, _).
IDCGroup(@place("", 0), category, i) :- option("entryPoint", "_filter_results_by_excluded_paths"), option_item("resultCategories", i, category).
IDCItem(@place("", 0), category, sid, i) :- option("entryPoint", "_filter_results_by_excluded_paths"), idc_supplied(category, sid, i), !idc_excluded(sid).
.decl idc_selected_category(i: number, category: symbol)
idc_selected_category(i, category) :- option_item("categories", i, category).
idc_selected_category(i, category) :- !option_item("categories", _, _), idc_execute_category(i, category).
.decl idc_target_path(sid: symbol)
idc_target_path(sid) :- sg_symbol(_, sid, _, _, _, _, _, _, _, _), !option_item("paths", _, _).
idc_target_path(sid) :- sg_symbol(_, sid, _, _, _, raw, _, _, _, _), path = @replace_all(raw, "\\\\", "/"), option_item("paths", _, selected), normalized = @replace_all(@replace_all(selected, "\\\\", "/"), "/+\\z", ""), (path = normalized ; prefix(cat(normalized, "/"), path)).
.decl idc_method_allowed(sid: symbol)
idc_method_allowed(sid) :- sg_symbol(_, sid, _, _, kind, _, _, _, owner, _), (owner = 0 ; kind != "method").
idc_method_allowed(sid) :- sg_owner(sid, class), sg_reachable(class).
.decl idc_target(i: number, j: number, path: symbol, sid: symbol)
idc_target(i, j, path, sid) :- idc_selected_category(i, category), idc_supplied(category, sid, j), sg_symbol(_, sid, name, _, _, path, _, _, _, _), !prefix("_lower_", name), idc_target_path(sid), idc_method_allowed(sid).
IDCTarget(@place("", 0), path, sid, rank) :- option("entryPoint", "_collect_execute_targets"), idc_target(i, j, path, sid), rank = count : { idc_target(other, ordinal, _, _), (other < i ; (other = i, ordinal < j)) }.
'''


def analysis_relations():
    return r'''
.decl idc_protocol(sid: symbol)
idc_protocol(sid) :- sg_symbol(_, sid, name, _, _, _, _, _, _, _), suffix("Protocol", name).
idc_protocol(sid) :- sg_base_text(sid, base), contains("Protocol", base).
.decl idc_implicit(sid: symbol)
idc_implicit(sid) :- sg_symbol(_, sid, name, _, _, _, _, _, _, _), ((prefix("__", name), suffix("__", name)) ; name = "main" ; name = "pytest_configure" ; name = "pytest_collection_modifyitems").
idc_implicit(sid) :- sg_decorator(sid, decorator), (suffix("validator", decorator) ; suffix("field_validator", decorator) ; suffix("model_validator", decorator) ; suffix("overload", decorator)).
.decl idc_public_api(sid: symbol)
idc_public_api(sid) :- sg_symbol(_, sid, _, module, _, _, _, _, _, _), (prefix("data_layer.", module) ; prefix("api.", module)).
.decl idc_reachable(sid: symbol)
idc_reachable(sid) :- sg_reachable(sid).
idc_reachable(target) :- idc_reachable(sid), sg_annotation(sid, name), sg_symbol(_, target, name, _, _, _, _, _, _, _), sg_class(target), count : { sg_symbol(_, other, name, _, _, _, _, _, _, _), sg_class(other) } = 1.
.decl idc_abc_implementation(c: symbol)
idc_abc_implementation(c) :- sg_base(c, base, _), idc_reachable(base), sg_symbol(_, base, name, _, _, _, _, _, _, abstract), (abstract = 1 ; contains("ABC", name)).
idc_abc_implementation(c) :- sg_ancestor(c, base), idc_reachable(base), sg_symbol(_, base, _, _, _, _, _, _, _, 1).
.decl idc_abstract_override(sid: symbol)
idc_abstract_override(sid) :- sg_owner(sid, c), sg_symbol(_, sid, name, _, _, _, _, _, _, _), sg_ancestor(c, base), sg_method(base, name, method), sg_symbol(_, method, _, _, _, _, _, _, _, 1).
.decl idc_import_referenced(sid: symbol)
idc_import_referenced(sid) :- option_item("symbolIds", i, sid), option_item("symbolFqNames", i, fq), sg_imported(_, _, fq).
.decl idc_callback(sid: symbol)
idc_callback(sid) :- sg_symbol(_, sid, name, _, _, _, _, _, _, _), ((prefix("_create_", name), (suffix("_connection", name) ; suffix("_validator", name) ; suffix("_factory", name))) ; contains("register", @lower(name))).
.decl idc_unused_function(sid: symbol)
idc_unused_function(sid) :- sg_function(sid), sg_symbol(_, sid, _, _, _, _, _, _, _, 0), !idc_reachable(sid), !sg_exported(sid), !sg_property(sid), !idc_public_api(sid), !idc_implicit(sid), count : { sg_owner(sid, c), idc_protocol(c) } = 0.
.decl idc_unused_class(sid: symbol)
idc_unused_class(sid) :- sg_class(sid), !idc_reachable(sid), !sg_exported(sid), !idc_protocol(sid), !idc_public_api(sid), !idc_implicit(sid).
.decl idc_high_function(sid: symbol)
idc_high_function(sid) :- idc_unused_function(sid), !idc_import_referenced(sid), !idc_abstract_override(sid), !idc_callback(sid), sg_symbol(_, sid, name, _, _, _, _, _, _, _), !prefix("_lower_", name), count : { sg_owner(sid, c), idc_abc_implementation(c) } = 0.
.decl idc_high_class(sid: symbol)
idc_high_class(sid) :- idc_unused_class(sid), !idc_import_referenced(sid), !idc_abc_implementation(sid).
.decl idc_intent(sid: symbol)
idc_intent(sid) :- sg_class(sid), sg_symbol(_, sid, name, _, _, _, _, _, _, _), suffix("Intent", name).
idc_intent(sid) :- sg_class(sid), sg_base_text(sid, base), contains("ServerPlanStep", base).
.decl idc_grammar(sid: symbol)
idc_grammar(sid) :- sg_class(sid), !idc_protocol(sid), sg_symbol(_, sid, name, _, _, path, _, _, _, _), (contains("/l2100_sql_grammar/", @replace_all(path, "\\\\", "/")) ; suffix("Grammar", name) ; prefix("Sql", name)).
idc_grammar(sid) :- sg_class(sid), !idc_protocol(sid), sg_base_text(sid, base), (contains("Sql", base) ; contains("Grammar", base)).
.decl idc_category(i: number, name: symbol)
idc_category(0, "unused_functions"). idc_category(1, "unused_intents"). idc_category(2, "unused_grammar_classes"). idc_category(3, "unused_other_classes").
idc_category(4, "high_confidence_functions"). idc_category(5, "high_confidence_intents"). idc_category(6, "high_confidence_grammar_classes"). idc_category(7, "high_confidence_other_classes").
.decl idc_result(category: symbol, sid: symbol)
idc_result("unused_functions", sid) :- idc_unused_function(sid).
idc_result("unused_intents", sid) :- idc_unused_class(sid), idc_intent(sid).
idc_result("unused_grammar_classes", sid) :- idc_unused_class(sid), idc_grammar(sid).
idc_result("unused_other_classes", sid) :- idc_unused_class(sid), !idc_intent(sid), !idc_grammar(sid).
idc_result("high_confidence_functions", sid) :- idc_high_function(sid).
idc_result("high_confidence_intents", sid) :- idc_high_class(sid), idc_intent(sid).
idc_result("high_confidence_grammar_classes", sid) :- idc_high_class(sid), idc_grammar(sid).
idc_result("high_confidence_other_classes", sid) :- idc_high_class(sid), !idc_intent(sid), !idc_grammar(sid).
.decl idc_sorted(category: symbol, sid: symbol, ordinal: number)
idc_sorted(category, sid, ordinal) :- idc_result(category, sid), sg_symbol(i, sid, _, _, _, path, line, _, _, _), ordinal = count : { idc_result(category, other), sg_symbol(j, other, _, _, _, other_path, other_line, _, _, _), (@str_cmp(other_path, path) < 0 ; (other_path = path, other_line < line) ; (other_path = path, other_line = line, j < i)) }.
IDCGroup(@place("", 0), category, i) :- option("entryPoint", "analyze_dead_code"), idc_category(i, category).
IDCItem(@place("", 0), category, sid, ordinal) :- option("entryPoint", "analyze_dead_code"), idc_sorted(category, sid, ordinal), !idc_excluded(sid).
'''
