"""Translate all ownership clauses; canonical syntax is a shared native input."""
import json,re
from data_layer_rr_text import module,character_class
from data_layer_rr_plan_ownership_graph import supplied_ast


def plan_scope_ownership(source,target,header):
    original=module(source,'plan_scope_ownership');q=json.dumps
    text=re.sub(r'^\.scope include .*$', '.scope include "impl/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l*/**/*.py", "conform/**/*.py", "data_layer_testing/**/*.py"',header('plan_scope_ownership',target),flags=re.M)
    text+='#include "python/print.rr"\n'
    for rel,values in [('pso_runner',original.BASE_SCOPE_RUNNERS),('pso_base_method',original.BASE_SCOPE_METHODS),
      ('pso_terminal',original.BASE_TERMINAL_FALLBACKS),('pso_lifecycle',original.LIFECYCLE_NAMES),
      ('pso_owned_file',original.PLAN_SCOPE_OWNERSHIP_FILES),('pso_nested_terminal',original.NESTED_VERSIONED_TERMINALS),
      ('pso_writer',original.CALLBACK_OWNER_WRITERS),('pso_trigger',original.PLAN_SCOPE_OWNERSHIP_FACT.source_triggers),
      ('pso_default_scan',original.SCAN_DIRECTORIES),('pso_default_skip',original.SKIP_DIR_NAMES)]:
        text+=f'.decl {rel}(value: symbol)\n'+''.join(f'{rel}({q(v)}).\n' for v in sorted(values))
    layer='(?s)l'+character_class(str.isdecimal)+r'{4}_[^\n].*'
    text+='.decl pso_layer(name: symbol)\npso_layer(n) :- option_item("topDirectories", _, n), match('+q(layer)+', n).\n'
    text+='pso_layer(n) :- doc(_, p, _), match_all("(?<root>[^/]+)/.*", p), n = root, match('+q(layer)+', n).\n'
    text+=r'''
.decl pso_native()
pso_native() :- !option("entryPoint", "collector").
.decl pso_api_root(path: symbol, rank: number)
pso_api_root("impl", 0). pso_api_root("data_layer", 1). pso_api_root("_internal", 2).
pso_api_root(p, 3 + i) :- option_item("topDirectories", i, p), pso_layer(p).
pso_api_root(p, 3 + n + i) :- option_item("scanDirectories", i, p), n = count : { option_item("topDirectories", _, _) }.
.decl pso_selected(i: number, path: symbol, rank: number)
pso_selected(i, p, rank) :- option_item("availablePaths", i, p), pso_api_root(r, _), prefix(cat(r, "/"), p),
 rank = min k : { pso_api_root(root, k), prefix(cat(root, "/"), p) },
 !pso_skipped(p).
.decl pso_skipped(path: symbol)
pso_skipped(p) :- option_item("availablePaths", _, p), option_item("skipDirectories", _, skip), contains(cat("/", skip, "/"), cat("/", p, "/")).
.violation Selected(at: span, index: number, path: symbol, root_rank: number) code "PSO_FILE" "{path}"
Selected(@place("<inventory>", 1), i, p, rank) :- option("entryPoint", "selection"), pso_selected(i, p, rank).

.decl pso_document(d: number, path: symbol)
pso_document(d, p) :- pso_native(), doc(d, p, _), option("entryPoint", entry),
 (entry = "lint" ; entry = "_lint_file" ; entry = "_source_facts").
pso_document(d, p) :- pso_native(), doc(d, p, _), !option("entryPoint", _), !pso_hidden(d),
 (prefix("impl/", p) ; prefix("data_layer/", p) ; prefix("_internal/", p) ;
  pso_layer(root), prefix(cat(root, "/"), p) ; pso_default_scan(root), prefix(cat(root, "/"), p)).
.decl pso_hidden(d: number)
pso_hidden(d) :- path_part(d, name), (name = ".git" ; name = ".venv" ; name = "venv" ; name = "__pycache__" ; name = "tmp").
.decl pso_raw(d: number, source: symbol)
pso_raw(d, t) :- pso_native(), pso_document(d, _), source(d, sp), text(sp, t).
.decl pso_enabled(d: number)
pso_enabled(d) :- pso_raw(d, t), pso_trigger(trigger), contains(trigger, t).
pso_enabled(d) :- pso_document(d, _), option("entryPoint", entry), (entry = "lint" ; entry = "_lint_file" ; entry = "_source_facts" ; entry = "collector").
.decl pso_valid(d: number)
pso_valid(d) :- pso_enabled(d), !syntax_error(d, _, _).
.decl pso_node(n: node, d: number, line: number, ordinal: number)
pso_node(n, d, l, i) :- pso_native(), docof(n, d), pso_valid(d), line(n, l), visit_index(n, i).
.decl pso_function(f: node, name: symbol)
pso_function(f, @nfkc(name)) :- pso_native(), kind(f, "function_definition"), def_name(f, name).
.decl pso_class(c: node, name: symbol)
pso_class(c, @nfkc(name)) :- pso_native(), kind(c, "class_definition"), def_name(c, name).
.decl pso_name(e: node, name: symbol)
pso_name(e, @nfkc(name)) :- pso_native(), unparen(e, n), kind(n, "identifier"), text(n, name).
.decl pso_attr(e: node, obj: node, name: symbol)
pso_attr(e, obj, @nfkc(name)) :- pso_native(), unparen(e, n), kind(n, "attribute"), field(n, "object", obj), field(n, "attribute", name_node), text(name_node, name).
.decl pso_call(c: node, fn: node)
pso_call(c, fn) :- pso_native(), kind(c, "call"), field(c, "function", fn).
.decl pso_leaf(e: node, name: symbol)
pso_leaf(e, name) :- pso_name(e, name).
pso_leaf(e, name) :- pso_attr(e, _, name).
.decl pso_inside(n: node, a: node)
pso_inside(n, a) :- pso_native(), ast_parent(n, a).
pso_inside(n, a) :- pso_native(), ast_parent(n, p), pso_inside(p, a).
.decl pso_owner(n: node, name: symbol)
pso_owner(n, @nfkc(name)) :- pso_native(), def_frame(n, f), def_name(f, name).
.decl pso_store(n: node)
pso_store(n) :- pso_native(), expr_ctx(n, "store").
.decl pso_parameter(f: node, p: node, index: number, annotation: symbol)
pso_parameter(f, p, i, "") :- pso_native(), sig_param(f, p, _, i, "pos", ann, _), unset(ann).
pso_parameter(f, p, i, printed) :- pso_native(), sig_param(f, p, _, i, "pos", ann, _), !unset(ann), annotation_text(ann, printed).
.decl pso_method(c: node, f: node, name: symbol)
pso_method(c, f, name) :- pso_native(), pso_class(c, _), field(c, "body", body), body_stmt(body, st, _), defstmt(f, st), pso_function(f, name).
.decl pso_build(c: node, statement: node, annotation: symbol)
pso_build(c, a, printed) :- pso_native(), pso_class(c, _), field(c, "body", body), body_stmt(body, st, _),
 kind(st, "expression_statement"), child(st, a, _), kind(a, "assignment"), field(a, "left", target), pso_name(target, "build"), field(a, "type", ann), annotation_text(ann, printed).
.decl pso_call_print(c: node, text: symbol)
pso_call_print(c, s) :- pso_native(), call_text(c, s).
.decl pso_callee_print(c: node, text: symbol)
pso_callee_print(c, s) :- pso_native(), call_function_text(c, s).
.decl pso_first(f: node, p: node, annotation: symbol)
pso_first(f, p, a) :- pso_parameter(f, p, i, a), i = min index : { pso_parameter(f, _, index, _) }.
.decl pso_second(f: node, annotation: symbol)
pso_second(f, a) :- pso_parameter(f, _, i, a), i = min index : { pso_parameter(f, _, index, _), pso_parameter(f, _, first, _), first < index }.
.decl pso_expected(f: node, name: symbol, expected: symbol)
pso_expected(f, name, "AsyncPlanScope") :- pso_function(f, name), pso_runner(name), suffix("_async", name).
pso_expected(f, name, "SyncPlanScope") :- pso_function(f, name), pso_runner(name), !suffix("_async", name).
.decl pso_issue(d: number, line: number, code: symbol, message: symbol, stage: number, event: number, rank: number, witness: number)
pso_issue(d, l, "PSO001", "base scope runner has no scope parameter", 0, i, 0, i) :- pso_expected(f, _, _), pso_node(f, d, l, i), !pso_first(f, _, _).
pso_issue(d, l, "PSO001", cat(name, " must accept exactly ", expected, ", not ", shown), 0, i, 0, i) :- pso_expected(f, name, expected), pso_node(f, d, _, i),
 pso_first(f, p, annotation), pso_node(p, d, l, _), annotation != expected, (annotation = "", shown = "untyped" ; annotation != "", shown = annotation).
pso_issue(d, l, "PSO008", cat(name, " workflow parameter must preserve its return generic"), 0, i, 1, i) :- pso_expected(f, name, _), pso_node(f, d, l, i),
 pso_first(f, _, _), !pso_second(f, _).
pso_issue(d, l, "PSO008", cat(name, " workflow parameter must preserve its return generic"), 0, i, 1, i) :- pso_expected(f, name, _), pso_node(f, d, l, i),
 pso_first(f, _, _), pso_second(f, annotation), !contains("[", annotation).
pso_issue(d, l, "PSO001", cat(name, " scope parameter is broader than its exact nominal base scope"), 0, i, 2, i) :- pso_expected(f, name, _), pso_node(f, d, _, i),
 pso_first(f, p, annotation), pso_node(p, d, l, _), (contains("Any", annotation) ; contains("object", annotation) ; contains("Versioned", annotation) ; contains("|", annotation) ; contains("Union", annotation)).
pso_issue(d, l, "PSO002", cat(name, " must call one typed scope workflow method, not reflective dispatch"), 0, i, 3, j) :- pso_function(f, name), pso_runner(name),
 pso_node(f, d, _, i), pso_inside(c, f), pso_node(c, d, l, j), pso_call(c, fn), pso_leaf(fn, leaf), (leaf = "getattr" ; leaf = "hasattr" ; leaf = "callable").
pso_issue(d, l, "PSO003", cat(name, " must not use fallback terminal ", attr), 0, i, 4, j) :- pso_function(f, name), pso_runner(name), pso_node(f, d, _, i),
 pso_inside(c, f), pso_node(c, d, l, j), pso_call(c, fn), pso_attr(fn, _, attr), pso_terminal(attr).
pso_issue(d, l, "PSO005", cat(name, " must not define base workflow terminal ", method), 0, i, 5, j) :- pso_class(c, name),
 (name = "SyncVersionedPlanScope" ; name = "AsyncVersionedPlanScope"), pso_node(c, d, _, i), pso_method(c, f, method), pso_base_method(method), pso_node(f, d, l, j).
pso_issue(d, l, "PSO008", cat(name, ".build must be typed as Callable[[", expected, "], T]"), 0, i, 0, j) :- pso_class(c, name), pso_node(c, d, _, i),
 (name = "SyncReadPlanWorkflow" ; name = "SyncWritePlanWorkflow" ; name = "AsyncReadPlanWorkflow" ; name = "AsyncWritePlanWorkflow"),
 expected = cat(@slice(name, 0, strlen(name) - strlen("PlanWorkflow")), "Transaction"), pso_build(c, st, annotation), pso_node(st, d, l, j),
 (contains("Any", annotation) ; !contains("Callable[[", annotation) ; !contains(expected, annotation)).
pso_issue(d, l, "PSO004", "versioned helpers must not call base plan-scope workflow runners", 0, i, 0, i) :- pso_call(c, fn), pso_node(c, d, l, i), pso_leaf(fn, name), pso_runner(name),
 pso_document(d, path), (path = "conform/versioned_plan_scope_helpers.py" ; path = "data_layer_testing/versioning/query_helpers.py").
pso_issue(d, l, "PSO006", "nested versioned contexts must not become terminal owners", 0, i, 1, i) :- pso_call(c, fn), pso_node(c, d, l, i), pso_document(d, path),
 pso_owned_file(path), pso_owner(c, owner), contains("nested_versioned", owner), pso_leaf(fn, name), pso_nested_terminal(name).
pso_issue(d, l, "PSO007", "lifecycle state must be accessed explicitly, not through reflective defaults", 0, i, 2, i) :- pso_call(c, fn), pso_node(c, d, l, i), pso_document(d, path),
 pso_owned_file(path), pso_leaf(fn, name), (name = "getattr" ; name = "hasattr"), pso_call_print(c, text), pso_lifecycle(state), contains(state, text).
pso_issue(d, l, "PSO009", "base workflow code must not name versioned scope or transaction classes", 0, i, 3, i) :- pso_call(c, _), pso_node(c, d, l, i),
 pso_document(d, path), suffix("connection_plan_workflows.py", path), pso_call_print(c, text), contains("Versioned", text).
pso_issue(d, l, "PSO013", "consumers must not invent CTE publication contracts", 0, i, 4, i) :- pso_call(c, _), pso_node(c, d, l, i),
 pso_callee_print(c, fn), contains("imported_contract", fn), pso_call_print(c, text), contains("CtePublicationContract", text).
pso_issue(d, l, "PSO010", "_callback_versioned_txn ownership may only be written by enumerated owner functions", 0, i, 0, i) :-
 pso_attr(n, _, "_callback_versioned_txn"), pso_store(n), pso_node(n, d, l, i), !pso_owner_allowed(n).
.decl pso_owner_allowed(n: node)
pso_owner_allowed(n) :- pso_owner(n, name), pso_writer(name).

.decl pso_raw_pattern(index: number, code: symbol, present: symbol, locate: symbol, requires: symbol, message: symbol)
'''
    raw=[('PSO011','if not txn.is_committed and not txn.is_rolled_back and not txn.did_publish_branch','if not txn.is_committed','','versioned helper commit decisions must enumerate all terminal states explicitly')]
    for pattern in ['execute_read_plan_workflow_async(\n            conn,\n            SyncReadPlanWorkflow','execute_write_plan_workflow_async(\n            conn,\n            SyncWritePlanWorkflow',
      'run_read_plan_workflow_async(\n            self._async_conn(),\n            SyncReadPlanWorkflow','run_write_plan_workflow_async(\n            self._async_conn(),\n            SyncWritePlanWorkflow',
      'execute_read_plan_workflow(\n        conn,\n        AsyncReadPlanWorkflow','execute_write_plan_workflow(\n        conn,\n        AsyncWritePlanWorkflow']:
        raw.append(('PSO012',pattern,pattern,'','sync and async workflow pairs must use matching mode-specific workflow classes'))
    raw.append(('PSO013','imported_contract = imported_contract_fn','imported_contract = imported_contract_fn','else None','imported CTE dependencies must fail without a producer publication contract'))
    for i,values in enumerate(raw):text+=f'pso_raw_pattern({i}, '+', '.join(q(v) for v in values)+').\n'
    for i,(_,_,locate,_,_) in enumerate(raw):
        pattern=re.escape(locate).replace('\\\n','\\n')
        text+=f'pso_raw_offset(d, {i}, a) :- pso_raw(d, t), match_all({q(pattern)}, t, _, a, _).\n'
    text+=r'''
.decl pso_raw_offset(d: number, pattern: number, offset: number)
pso_issue(d, strlen(@replace_all(@slice(text, 0, at), "[^\\n]", "")) + 1, code, message, 1, index, 0, at) :-
 pso_raw(d, text), pso_valid(d), pso_raw_pattern(index, code, present, _, requires, message), contains(present, text),
 (requires = "" ; contains(requires, text)), at = min offset : { pso_raw_offset(d, index, offset) }.
'''
    # Each code retains every event witness, including identical messages from nested runner inventories.
    for i in range(14):
        code=f'PSO{i:03}'
        text+=f'.violation Ownership{i}(at: span, message: symbol, stage: number, event: number, rank: number, witness: number) code "{code}" "{{message}}"\n'
        text+=f'Ownership{i}(@place(path, line), message, stage, event, rank, witness) :- pso_issue(d, line, "{code}", message, stage, event, rank, witness), pso_document(d, path).\n'
    text+=r'''
pso_issue(d, 1, "PSO000", cat(message, " (", filename, ", line ", to_string(line), ")"), 0, 0, 0, 0) :- pso_document(d, _), pso_enabled(d), syntax_error(d, line, message), file_name(d, filename).
'''
    return text+supplied_ast()
