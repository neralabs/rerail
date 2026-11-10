"""Generate ServerPlan's static target-architecture checks from pinned configuration."""
import json
import re

from data_layer_rr_text import module


def serverplan_target_architecture(source, target, header):
    original = module(source, 'serverplan_target_architecture')
    text = header('serverplan_target_architecture', target)
    excluded = ('.git', '.venv', 'venv', '__pycache__', 'tmp', 'test', 'tests')
    text = re.sub(r'^\.scope exclude .*$',
                  '.scope exclude ' + ', '.join(json.dumps(f'**/{part}/**') for part in excluded),
                  text, flags=re.M)
    tables = {
        'spta_root': (('path',), ((p + '/',) for p in original._SCAN_DIRS)),
        'spta_skip': (('component',), ((part,) for part in excluded)),
        'spta_required': (('path', 'name'), ((p, n) for p, names in original._REQUIRED_SYMBOLS.items() for n in names)),
        'spta_required_field': (('path', 'class', 'name'), ((p, c, n) for (p, c), names in original._REQUIRED_CLASS_FIELDS.items() for n in names)),
        'spta_immutable': (('path', 'class'), original._REQUIRED_IMMUTABLE_CLASSES),
        'spta_required_member': (('path', 'enum', 'name'), ((p, c, n) for (p, c), names in original._REQUIRED_ENUM_MEMBERS.items() for n in names)),
        'spta_required_variant': (('path', 'alias', 'name'), ((p, a, n) for (p, a), names in original._REQUIRED_UNION_MEMBERS.items() for n in names)),
        'spta_legacy': (('name',), ((n,) for n in original._LEGACY_SYMBOLS)),
        'spta_backend': (('name',), ((n,) for n in original._BACKEND_PARTS)),
        'spta_forbidden_import': (('path', 'prefix'), ((p, x) for p, values in original._PROHIBITED_IMPORTS.items() for x in values)),
    }
    for relation, (columns, rows) in tables.items():
        text += f'.decl {relation}(' + ', '.join(c + ': symbol' for c in columns) + ')\n'
        text += ''.join(f'{relation}(' + ', '.join(map(json.dumps, row)) + ').\n' for row in sorted(rows))
    return text + r'''
.decl spta_name(n: node, name: symbol)
spta_name(n, @nfkc(x)) :- unparen(n, e), ast_name(e, x).
spta_name(n, @nfkc(x)) :- unparen(n, e), kind(e, "attribute"), field(e, "attribute", a), text(a, x).
.decl spta_names(n: node, name: symbol)
spta_names(n, @nfkc(x)) :- ast_name(n, x).
spta_names(n, @nfkc(x)) :- within(a, n), ast_name(a, x).
.decl spta_doc(d: number, path: symbol)
spta_doc(d, p) :- doc(d, p, _), doc_role(d, "primary"), spta_root(root), prefix(root, p),
    suffix(".py", p), !spta_skipped(d), !syntax_error(d, _, _).
.decl spta_skipped(d: number)
spta_skipped(d) :- path_part(d, part), spta_skip(part).
.decl spta_node(n: node, path: symbol, line: number, order: number)
spta_node(n, p, l, i) :- docof(n, d), spta_doc(d, p), line(n, l), visit_index(n, i).

.decl spta_top(n: node)
spta_top(n) :- ast_parent(n, m), kind(m, "module").
.decl spta_class(n: node, path: symbol, name: symbol)
spta_class(n, p, @nfkc(name)) :- kind(n, "class_definition"), def_name(n, name), spta_node(n, p, _, _),
    !inside(n, _, "class_definition"), !def_frame(n, _).
.decl spta_latest_class(n: node, path: symbol, name: symbol)
spta_latest_class(n, p, c) :- spta_class(n, p, c), !spta_later_class(n, p, c).
.decl spta_later_class(n: node, path: symbol, name: symbol)
spta_later_class(n, p, c) :- spta_class(n, p, c), spta_class(other, p, c), earlier(n, other).
.decl spta_target(a: node, n: node)
spta_target(a, n) :- assign_target(a, n).
spta_target(a, n) :- kind(a, "augmented_assignment"), field(a, "left", n).
.decl spta_value(a: node, v: node)
spta_value(a, v) :- assign_value(a, v).
spta_value(a, v) :- kind(a, "augmented_assignment"), field(a, "right", v).
.decl spta_top_assignment(a: node, path: symbol, name: symbol)
spta_top_assignment(a, p, x) :- spta_top(a), spta_node(a, p, _, _), spta_target(a, n), spta_name(n, x).
.decl spta_symbol(path: symbol, name: symbol)
spta_symbol(p, c) :- spta_class(_, p, c).
spta_symbol(p, @nfkc(x)) :- spta_top(f), kind(f, "function_definition"), def_name(f, x), spta_node(f, p, _, _).
spta_symbol(p, x) :- spta_top_assignment(_, p, x).

.decl spta_class_member(c: node, n: node)
spta_class_member(c, n) :- kind(c, "class_definition"), ast_parent(n, c).
.decl spta_field(c: node, name: symbol, a: node)
spta_field(c, @nfkc(x), a) :- spta_class_member(c, n), assign_ann(n, a), assign_target(n, t),
    unparen(t, q), ast_name(q, x).
.decl spta_member(c: node, name: symbol)
spta_member(c, @nfkc(x)) :- spta_class_member(c, a), kind(a, "assignment"), !assign_ann(a, _),
    assign_target(a, n), unparen(n, q), ast_name(q, x).
.decl spta_base(c: node, name: symbol)
spta_base(c, x) :- field(c, "superclasses", bases), child(bases, b, _), !kind(b, "keyword_argument"), spta_names(b, x).

.decl spta_dataclass(c: node, args: node, i: number)
spta_dataclass(c, args, i) :- decorator(c, e), unparen(e, call), kind(call, "call"),
    field(call, "function", f), spta_name(f, "dataclass"), field(call, "arguments", args), visit_index(call, i).
.decl spta_frozen(c: node)
spta_frozen(c) :- spta_dataclass(c, args, i), i = min j : { spta_dataclass(c, _, j) },
    spta_true_kw(args, "frozen"), spta_true_kw(args, "slots").
.decl spta_true_kw(args: node, name: symbol)
spta_true_kw(args, x) :- spta_kw(args, x, v, i), i = max j : { spta_kw(args, x, _, j) },
    unparen(v, e), kind(e, "true").
.decl spta_kw(args: node, name: symbol, value: node, i: number)
spta_kw(args, @nfkc(x), v, i) :- child(args, kw, i), kind(kw, "keyword_argument"),
    field(kw, "name", n), text(n, x), field(kw, "value", v).

.decl spta_assigned_variant(path: symbol, alias: symbol, variant: symbol)
spta_assigned_variant(p, x, v) :- spta_top_assignment(a, p, x), spta_value(a, expr), spta_names(expr, v),
    !spta_later_assignment(a, p, x).
.decl spta_later_assignment(a: node, path: symbol, name: symbol)
spta_later_assignment(a, p, x) :- spta_top_assignment(a, p, x), spta_top_assignment(b, p, x),
    spta_value(b, _), earlier(a, b).
.decl spta_method(c: node, f: node, name: symbol)
spta_method(c, f, @nfkc(x)) :- spta_class_member(c, f), kind(f, "function_definition"), def_name(f, x).
.decl spta_last_method(c: node, f: node, name: symbol)
spta_last_method(c, f, x) :- spta_method(c, f, x), !spta_later_method(c, f, x).
.decl spta_later_method(c: node, f: node, name: symbol)
spta_later_method(c, f, x) :- spta_method(c, f, x), spta_method(c, g, x), earlier(f, g).
.decl spta_return(f: node, name: symbol)
spta_return(f, x) :- field(f, "return_type", t), spta_names(t, x).
.decl spta_parameter(f: node, name: symbol)
spta_parameter(f, x) :- sig_param(f, _, _, _, k, t, _), k != "vararg", k != "kwarg", spta_names(t, x).
.decl spta_planning()
spta_planning() :- spta_latest_class(c, "l2070_batching/step.py", "ServerPlanStep"),
    spta_last_method(c, f, "planning_contract"), spta_return(f, "StepPlanningContract").
.decl spta_application()
spta_application() :- spta_latest_class(c, "l2070_batching/physical_placement.py", "ApplicationPlacementUnit"),
    spta_field(c, "application", a), !spta_later_field(c, "application", a), spta_names(a, "RuntimeApplication").
.decl spta_later_field(c: node, name: symbol, a: node)
spta_later_field(c, x, a) :- spta_field(c, x, a), spta_field(c, x, b), earlier(a, b).
.decl spta_executor()
spta_executor() :- spta_latest_class(c, "l2070_batching/physical_executor.py", "BackendExecutor"),
    spta_last_method(c, f, "execute_program"), spta_parameter(f, "PhysicalTransportProgram"), spta_return(f, "BackendExecutionReceipt").

.decl spta_shape()
spta_shape() :- !option("requiredShape", "false").
.decl spta_report(path: symbol, line: number, code: symbol, name: symbol, message: symbol, order: number)
spta_report(p, 1, "SPTA001", x, cat("required target contract is missing from ", p), 0) :-
    spta_shape(), spta_required(p, x), !spta_symbol(p, x).
spta_report("l2070_batching/step.py", 1, "SPTA002", "ServerPlanStep.planning_contract",
    "semantic producers must expose one typed planning contract", 0) :- spta_shape(), !spta_planning().
spta_report("l2070_batching/physical_placement.py", 1, "SPTA003", "ApplicationPlacementUnit.application",
    "physical placement must select a closed runtime application", 0) :- spta_shape(), !spta_application().
spta_report("l2070_batching/physical_executor.py", 1, "SPTA011", "BackendExecutor.execute_program",
    "backend execution must accept a physical program and return a typed receipt", 0) :- spta_shape(), !spta_executor().

.decl spta_missing_field(path: symbol, class: symbol, field: symbol)
spta_missing_field(p, x, f) :- spta_required_field(p, x, f), spta_latest_class(c, p, x), !spta_field(c, f, _).
spta_report(p, 1, "SPTA013", x, cat("target contract is missing fields: ", missing), 0) :-
    spta_shape(), spta_missing_field(p, x, _), missing = join f by f sep ", " : { spta_missing_field(p, x, f) }.
spta_report(p, 1, "SPTA013", x, "target contract must be a frozen slots dataclass", 1) :-
    spta_shape(), spta_immutable(p, x), spta_latest_class(c, p, x), !spta_frozen(c).
.decl spta_enum_problem(path: symbol, enum: symbol, order: number, message: symbol)
spta_enum_problem(p, x, 0, "must inherit StrEnum") :- spta_required_member(p, x, _),
    spta_latest_class(c, p, x), !spta_base(c, "StrEnum").
.decl spta_missing_member(path: symbol, enum: symbol, member: symbol)
spta_missing_member(p, x, n) :- spta_required_member(p, x, n), spta_latest_class(c, p, x), !spta_member(c, n).
spta_enum_problem(p, x, 1, cat("missing members: ", missing)) :- spta_missing_member(p, x, _),
    missing = join n by n sep ", " : { spta_missing_member(p, x, n) }.
spta_report(p, 1, "SPTA013", x, message, 2) :- spta_shape(), spta_enum_problem(p, x, _, _),
    message = join part by i sep "; " : { spta_enum_problem(p, x, i, part) }.
.decl spta_missing_variant(path: symbol, alias: symbol, variant: symbol)
spta_missing_variant(p, x, v) :- spta_required_variant(p, x, v), spta_symbol(p, x), !spta_assigned_variant(p, x, v).
spta_report(p, 1, "SPTA013", x, cat("closed union is missing variants: ", missing), 3) :-
    spta_shape(), spta_missing_variant(p, x, _), missing = join v by v sep ", " : { spta_missing_variant(p, x, v) }.

spta_report(p, l, "SPTA004", x, "legacy semantic batching/strategy symbol must be removed", i) :-
    kind(c, "class_definition"), def_name(c, raw), x = @nfkc(raw), spta_legacy(x), spta_node(c, p, l, i).
.decl spta_import_item(n: node, name: symbol)
spta_import_item(n, x) :- kind(n, "dotted_name"), text(n, x).
spta_import_item(n, x) :- kind(n, "aliased_import"), field(n, "name", a), text(a, x).
spta_report("l2070_batching/executor.py", l, "SPTA005", name, "physical executor must not import concrete domain steps", j) :-
    from_module_raw(n, m), (contains(".steps.", m) ; suffix(".steps", m)), field(n, "name", a), spta_import_item(a, name),
    (suffix("Step", name) ; suffix("Intent", name) ; suffix("Strategy", name)), spta_node(n, "l2070_batching/executor.py", l, _), visit_index(a, j).
spta_report("l2070_batching/executor.py", l, "SPTA006", @nfkc(name), "physical executor path must consume physical programs, not semantic steps", i) :-
    kind(f, "function_definition"), def_name(f, name), spta_parameter(f, "ServerPlanStep"), spta_node(f, "l2070_batching/executor.py", l, i).
.decl spta_condition(owner: node, condition: node)
spta_condition(n, c) :- py_if(n, c, _).
spta_condition(n, c) :- kind(n, "conditional_expression"), child(n, c, 1).
spta_condition(n, c) :- kind(n, "match_statement"), field(n, "subject", c).
spta_condition(n, g) :- kind(n, "match_statement"), field(n, "body", b), child(b, c, _), kind(c, "case_clause"), field(c, "guard", g).
.decl spta_call(n: node, name: symbol)
spta_call(n, x) :- kind(n, "call"), field(n, "function", f), spta_name(f, x).
spta_report(p, l, "SPTA007", "step_tags", "tags are diagnostic and must not determine semantics or placement", i) :-
    spta_condition(n, c), spta_call(call, "step_tags"), (call = c ; within(call, c)), spta_node(n, p, l, i).
.decl spta_positional(args: node, arg: node, i: number)
spta_positional(args, a, j) :- spta_positional_child(args, a, i),
    j = count : { spta_positional_child(args, _, k), k < i }.
.decl spta_positional_child(args: node, arg: node, i: number)
spta_positional_child(args, a, i) :- child(args, a, i), !kind(a, "keyword_argument"), !kind(a, "dictionary_splat"),
    !kind(a, "comment"), !kind(a, "line_continuation").
.decl spta_strategy_arg(n: node, name: symbol, arg: node, path: symbol, line: number, order: number)
spta_strategy_arg(n, x, a, p, l, i) :- spta_call(n, x), (x = "getattr" ; x = "isinstance"),
    spta_node(n, p, l, i), prefix("impl/common/l2070_server_plan/strategies/", p),
    field(n, "arguments", args), spta_positional(args, a, 1).
spta_report(p, l, "SPTA008", "getattr", "generic strategy must not inspect private grammar state", i) :-
    spta_strategy_arg(_, "getattr", a, p, l, i), unparen(a, e), plain_str(e, x),
    (x = "_stmt" ; x = "_grammar" ; x = "grammar").
spta_report(p, l, "SPTA008", "isinstance", "result and placement requirements must be producer-declared", i) :-
    spta_strategy_arg(_, "isinstance", a, p, l, i), spta_names(a, x),
    (prefix("Sql", x) ; suffix("Grammar", x) ; suffix("Stmt", x)).
spta_report(p, l, "SPTA009", x, "generic planning must not define backend-named transport policy", i) :-
    (kind(n, "class_definition") ; kind(n, "function_definition")), def_name(n, raw), x = @nfkc(raw), lower = @lower(x),
    spta_node(n, p, l, i), (prefix("impl/common/l2070_server_plan/", p) ; prefix("l2070_batching/", p)),
    !prefix("impl/common/l2070_server_plan/codegen/", p), spta_backend(b), contains(b, lower),
    (contains("batch", lower) ; contains("coalesce", lower) ; contains("strategy", lower) ; contains("transport", lower)).
.decl spta_account(a: node, name: symbol)
spta_account(a, x) :- spta_target(a, n), spta_name(n, x), (x = "driver_round_trips" ; x = "query_rtts" ; x = "round_trips").
.decl spta_receipt(v: node)
spta_receipt(v) :- unparen(v, e), kind(e, "attribute"), field(e, "object", o), unparen(o, n), ast_name(n, raw),
    name = @nfkc(raw), (name = "receipt" ; name = "execution_receipt").
spta_report(p, l, "SPTA010", x, "routes must consume authoritative backend receipt counts", i) :-
    spta_account(a, _), x = min n : { spta_account(a, n) }, spta_node(a, p, l, i),
    prefix("l9900_auto_api/", p), spta_value(a, v), !spta_receipt(v).
.decl spta_dependency(n: node, name: symbol, i: number)
spta_dependency(n, name, i) :- from_module_raw(n, name), visit_index(n, i).
spta_dependency(n, name, j) :- kind(n, "import_statement"), field(n, "name", a), spta_import_item(a, name), visit_index(a, j).
spta_report(p, l, "SPTA012", name, "target contract module violates its dependency boundary", i) :-
    spta_dependency(n, name, i), spta_node(n, p, l, _), spta_forbidden_import(p, forbidden), prefix(forbidden, name).

.violation TargetArchitecture(at: span, code: symbol, name: symbol, message: symbol, order: number) code {code} "{message}"
TargetArchitecture(@place(p, l), code, x, message, i) :- spta_report(p, l, code, x, message, i), !spta_helper().
.decl spta_helper()
spta_helper() :- option("entryPoint", "_frozen_slots").
spta_helper() :- option("entryPoint", "_symbol_names").
.violation TargetFrozen(at: span) code "SPTA_FROZEN" "frozen slots dataclass"
TargetFrozen(@place("", 0)) :- option("entryPoint", "_frozen_slots"), spta_frozen(c), spta_top(c).
.decl spta_bad_symbol()
spta_bad_symbol() :- option_item("itemKinds", _, k), k != "str".
.decl spta_symbol_list()
spta_symbol_list() :- option("inputKind", "list"), !spta_bad_symbol().
.violation TargetSymbol(at: span, value: symbol) code "SPTA_SYMBOL" "{value}"
TargetSymbol(@place("", 0), x) :- option("entryPoint", "_symbol_names"), spta_symbol_list(), option_item("items", _, x).
.violation TargetBadSymbols(at: span) code "SPTA_BAD_SYMBOLS" "expected a list of strings"
TargetBadSymbols(@place("", 0)) :- option("entryPoint", "_symbol_names"), !spta_symbol_list().
'''
