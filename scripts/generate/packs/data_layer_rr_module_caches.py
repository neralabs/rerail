"""Port cache assignments, decorators, selection and allowlist validation to native relations."""
import json
import re
from data_layer_rr_text import module, character_class

ALLOWLIST = 'scripts/allowlists/lint_module_level_caches_allowlist.json'


def module_level_caches(source, target, header):
    original = module(source, 'module_level_caches')
    q = json.dumps
    text = header('module_level_caches', target)
    text = re.sub(r'^\.scope include .*$', '.scope include "impl/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l*_*/**/*.py"', text, flags=re.M)
    text = re.sub(r'^\.scope exclude .*$', '.scope exclude "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**", "**/tests/**", "**/test/**"', text, flags=re.M)
    for relation, values in [('mc_skip_part', original.SKIP_PARTS), ('mc_test_part', original.TEST_PARTS),
            ('mc_cache_decorator_name', original.CACHE_DECORATORS), ('mc_constructor_name', original.CACHE_CONSTRUCTOR_NAMES),
            ('mc_managed_name', original.MANAGED_CACHE_CONSTRUCTOR_NAMES), ('mc_token', original.CACHE_NAME_TOKENS),
            ('mc_trigger', original.MODULE_LEVEL_CACHES_EXTRACTOR.source_triggers)]:
        text += f'.decl {relation}(value: symbol)\n'
        text += ''.join(f'{relation}({q(v)}).\n' for v in sorted(values))
    changing = character_class(lambda c: c.upper() != c)
    text += '.decl mc_upper_changes(name: symbol)\nmc_upper_changes(name) :- mc_assignment(_, _, name, _, _), match('+q('(?s).*'+changing+'.*')+', name).\n'
    text += '.decl mc_layer_dir(name: symbol)\nmc_layer_dir(name) :- option_item("directoryNames", _, name), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^\n].*')+', name).\n'
    text += '.decl mc_production_layer(path: symbol)\nmc_production_layer(path) :- mc_path(_, path), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', path).\n'
    text += projection()+selection()+assignments()+outputs()
    for relation, field in [('mc_al_path','path'), ('mc_al_pattern','pattern_type'), ('mc_al_name','name'),
                            ('mc_al_classification','classification'), ('mc_al_reason','reason')]:
        text += f'.decl {relation}(i: number, value: symbol)\n.input {relation}(IO=json, filename={q(ALLOWLIST)}, path={q("/entries/*/"+field)})\n'
    for relation, pointer, columns in [('mc_al_root_type', '', 'kind: symbol'),
            ('mc_al_entries_type', '/entries', 'kind: symbol'), ('mc_al_item_type', '/entries/*', 'i: number, kind: symbol')]:
        text += f'.decl {relation}({columns})\n.input {relation}(IO=json, filename={q(ALLOWLIST)}, path={q(pointer)}, projection="type")\n'
    whitespace = character_class(str.isspace)
    text += '.decl mc_al_blank_classification(i: number)\nmc_al_blank_classification(i) :- mc_al_classification(i, value), match('+q(whitespace+'*')+', value).\n'
    text += '.decl mc_al_blank_reason(i: number)\nmc_al_blank_reason(i) :- mc_al_reason(i, value), match('+q(whitespace+'*')+', value).\n'
    return text+allowlist()


def projection():
    return r'''
.decl mc_graph()
mc_graph() :- option_item("astKinds", 0, _).
.decl mc_gkind(n: node, k: symbol)
mc_gkind(n, k) :- option_item("astKinds", n, k).
.decl mc_edge(p: node, field: symbol, c: node)
mc_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl mc_parent(n: node, p: node)
mc_parent(n, p) :- !mc_graph(), ast_parent(n, p).
mc_parent(n, p) :- mc_edge(p, _, n).
.decl mc_list_length(n: node, field: symbol, length: number)
mc_list_length(to_number(owner), field, to_number(length)) :- option_item("astListOwners", i, owner), option_item("astListFields", i, field), option_item("astListLengths", i, length).
.decl mc_ancestor(n: node, a: node)
mc_ancestor(n, p) :- mc_parent(n, p).
mc_ancestor(n, a) :- mc_parent(n, p), mc_ancestor(p, a).
.decl mc_function(f: node, name: symbol)
mc_function(f, @nfkc(name)) :- !mc_graph(), kind(f, "function_definition"), def_name(f, name).
mc_function(f, name) :- mc_gkind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl mc_class(c: node, name: symbol)
mc_class(c, @nfkc(name)) :- !mc_graph(), kind(c, "class_definition"), def_name(c, name).
mc_class(c, name) :- mc_gkind(c, "ClassDef"), option_item("astNames", c, name).
.decl mc_lambda(n: node)
mc_lambda(n) :- !mc_graph(), kind(n, "lambda").
mc_lambda(n) :- mc_gkind(n, "Lambda").
.decl mc_frame(n: node)
mc_frame(n) :- mc_function(n, _).
mc_frame(n) :- mc_class(n, _).
mc_frame(n) :- mc_lambda(n).
.decl mc_in_frame(n: node)
mc_in_frame(n) :- mc_ancestor(n, f), mc_frame(f).
.decl mc_name(n: node, name: symbol)
mc_name(n, @nfkc(name)) :- !mc_graph(), unparen(n, e), kind(e, "identifier"), text(e, name).
mc_name(n, name) :- mc_gkind(n, "Name"), option_item("astIds", n, name).
.decl mc_attribute(n: node, object: node, name: symbol)
mc_attribute(n, object, @nfkc(name)) :- !mc_graph(), unparen(n, a), kind(a, "attribute"), field(a, "object", object), field(a, "attribute", nm), text(nm, name).
mc_attribute(n, object, name) :- mc_gkind(n, "Attribute"), mc_edge(n, "value", object), option_item("astAttrs", n, name).
.decl mc_call(n: node, fn: node)
mc_call(n, fn) :- !mc_graph(), unparen(n, c), kind(c, "call"), field(c, "function", fn).
mc_call(n, fn) :- mc_gkind(n, "Call"), mc_edge(n, "func", fn).
.decl mc_call_leaf(n: node, name: symbol)
mc_call_leaf(n, name) :- mc_name(n, name).
mc_call_leaf(n, name) :- mc_attribute(n, _, name).
.decl mc_has_argument(n: node)
mc_has_argument(n) :- !mc_graph(), mc_call(n, _), field(n, "arguments", args), child(args, a, _), !kind(a, "comment").
mc_has_argument(n) :- mc_gkind(n, "Call"), mc_list_length(n, kind, length), (kind = "args" ; kind = "keywords"), length > 0.
.decl mc_false(n: node)
mc_false(n) :- !mc_graph(), unparen(n, e), kind(e, "false").
mc_false(n) :- mc_gkind(n, "Constant"), option_item("astValueTypes", n, "bool"), option_item("astLiteralJson", n, "\"False\"").
.decl mc_registration_keyword(c: node, value: node, i: number)
mc_registration_keyword(c, value, i) :- !mc_graph(), mc_call(c, _), field(c, "arguments", args), child(args, kw, i), kind(kw, "keyword_argument"), field(kw, "name", nm), text(nm, name), @nfkc(name) = "register_with_manager", field(kw, "value", value).
mc_registration_keyword(c, value, to_number(i)) :- mc_gkind(c, "Call"), mc_edge(c, "keywords", kw), option_item("astKeywords", kw, "register_with_manager"), option_item("astOrdinals", kw, i), mc_edge(kw, "value", value).
.decl mc_registration_disabled(c: node)
mc_registration_disabled(c) :- mc_registration_keyword(c, v, i), i = min j : { mc_registration_keyword(c, _, j) }, mc_false(v).
.decl mc_empty(n: node, kind: symbol)
.decl mc_container_child(n: node)
mc_container_child(n) :- !mc_graph(), child(n, c, _), !kind(c, "comment").
mc_empty(n, "dict") :- !mc_graph(), unparen(n, d), kind(d, "dictionary"), !mc_container_child(d).
mc_empty(n, "dict") :- mc_gkind(n, "Dict"), mc_list_length(n, "keys", 0).
mc_empty(n, "list") :- !mc_graph(), unparen(n, d), kind(d, "list"), !mc_container_child(d).
mc_empty(n, "list") :- mc_gkind(n, "List"), mc_list_length(n, "elts", 0).
mc_empty(n, name) :- mc_call(n, fn), mc_name(fn, name), (name = "dict" ; name = "set" ; name = "list"), !mc_has_argument(n).
.decl mc_list(n: node)
mc_list(n) :- !mc_graph(), unparen(n, l), kind(l, "list").
mc_list(n) :- mc_gkind(n, "List").
.decl mc_assignment(a: node, target: node, name: symbol, value: node, suborder: number)
.decl mc_target_name(n: node, name: symbol)
mc_target_name(n, name) :- mc_name(n, name).
mc_target_name(n, name) :- !mc_graph(), kind(n, "tuple_pattern"), !op(n, ","), child(n, c, _), !kind(c, "comment"),
 1 = count : { child(n, value, _), !kind(value, "comment") }, mc_target_name(c, name).
mc_assignment(a, t, name, v, i) :- !mc_graph(), assign_target(a, t), mc_target_name(t, name), assign_value(a, raw), unparen(raw, v), visit_index(t, i).
mc_assignment(a, t, name, v, to_number(i)) :- mc_gkind(a, "Assign"), mc_edge(a, "targets", t), mc_name(t, name), mc_edge(a, "value", v), option_item("astOrdinals", t, i).
mc_assignment(a, t, name, v, 0) :- mc_gkind(a, "AnnAssign"), mc_edge(a, "target", t), mc_name(t, name), mc_edge(a, "value", v).
.decl mc_position(n: node, d: number, line: number, column: number, visit: number)
mc_position(n, d, l, c, i) :- !mc_graph(), node(d, n, _, _, _), line(n, l), col(n, c), visit_index(n, i).
mc_position(n, 0, to_number(l), to_number(c), n) :- mc_gkind(n, _), option_item("astLines", n, l), option_item("astColumns", n, c).
.decl mc_path(d: number, path: symbol)
mc_path(d, path) :- !mc_graph(), doc(d, physical, _), doc_role(d, "primary"), option_item("documentPaths", i, physical), option_item("scopePaths", i, path).
mc_path(d, path) :- !mc_graph(), doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
mc_path(0, path) :- mc_graph(), option("originalPath", path).
.decl mc_decorator(f: node, expr: node, i: number)
mc_decorator(f, e, i) :- !mc_graph(), decorator(f, e), visit_index(e, i).
mc_decorator(f, e, to_number(i)) :- mc_function(f, _), mc_edge(f, "decorator_list", e), option_item("astOrdinals", e, i).
.decl mc_cache_decorator(n: node, name: symbol)
mc_cache_decorator(n, name) :- mc_name(n, name), mc_cache_decorator_name(name).
mc_cache_decorator(n, cat("functools.", name)) :- mc_attribute(n, object, name), mc_name(object, "functools"), mc_cache_decorator_name(name).
mc_cache_decorator(n, name) :- mc_call(n, fn), mc_cache_decorator(fn, name).
'''


def selection():
    return r'''
.decl mc_default_dir(i: number, path: symbol)
mc_default_dir(0, "impl") :- option_item("directoryNames", _, "impl").
mc_default_dir(1, "data_layer") :- option_item("directoryNames", _, "data_layer").
mc_default_dir(2, "_internal") :- option_item("directoryNames", _, "_internal").
mc_default_dir(3 + count : { mc_layer_dir(earlier), @str_cmp(earlier, name) < 0 }, name) :- mc_layer_dir(name).
.decl mc_scan_dir(i: number, path: symbol)
mc_scan_dir(i, path) :- option("dirsProvided", "true"), option_item("scanDirs", i, path).
mc_scan_dir(i, path) :- !option("dirsProvided", "true"), mc_default_dir(i, path).
.decl mc_candidate(i: number, path: symbol)
mc_candidate(i, path) :- option_item("candidatePaths", i, path).
.decl mc_candidate_part(i: number, part: symbol)
mc_candidate_part(i, part) :- option_item("candidatePartOwners", j, owner), option_item("candidateParts", j, part), i = to_number(owner).
.decl mc_candidate_skip(i: number)
mc_candidate_skip(i) :- mc_candidate_part(i, part), mc_skip_part(part).
mc_candidate_skip(i) :- mc_candidate_part(i, part), mc_test_part(part), !option("includeTests", "true").
.decl mc_file(group: number, i: number, path: symbol)
mc_file(group, i, path) :- mc_candidate(i, path), mc_scan_dir(group, dir),
 option_item("candidateGroupOwners", j, owner), i = to_number(owner), option_item("candidateGroups", j, dir), !mc_candidate_skip(i).
.violation MCFile(at: span, group: number, i: number) code "MC_FILE" "selected"
MCFile(@place(path, 0), group, i) :- option("entryPoint", "_collect_files"), mc_file(group, i, path).
.violation MCFileCount(at: span, value: number) code "MC_FILE_COUNT" "{value}"
MCFileCount(@place("", 0), count : { mc_file(group, i, path) }) :- option("entryPoint", "_collect_files").
.violation MCDirectory(at: span, i: number, value: symbol) code "MC_DIRECTORY" "{value}"
MCDirectory(@place("", 0), i, path) :- option("entryPoint", "_scan_dirs"), mc_default_dir(i, path).
.decl mc_triggered(d: number)
mc_triggered(d) :- doc(d, _, _), source(d, sp), text(sp, src), mc_trigger(trigger), contains(trigger, src).
.decl mc_selected(d: number)
.decl mc_production_hidden(d: number)
mc_production_hidden(d) :- path_part(d, part), (part = ".git" ; part = ".venv" ; part = "venv" ; part = "__pycache__" ; part = "tmp" ; part = "tests" ; part = "test").
mc_selected(0) :- mc_graph().
mc_selected(d) :- !mc_graph(), mc_path(d, _), option("entryPoint", entry), entry != "_collect_files", entry != "_scan_dirs", mc_triggered(d), !syntax_error(d, _, _).
mc_selected(d) :- !mc_graph(), mc_path(d, path), !option("entryPoint", _), mc_triggered(d), !syntax_error(d, _, _), !mc_production_hidden(d),
 (prefix("impl/", path) ; prefix("data_layer/", path) ; prefix("_internal/", path) ; mc_production_layer(path)).
'''


def assignments():
    return r'''
.decl mc_outer_function(f: node)
mc_outer_function(f) :- mc_function(f, _), mc_ancestor(f, outer), mc_function(outer, _).
mc_outer_function(f) :- mc_function(f, _), mc_ancestor(f, outer), mc_lambda(outer).
.decl mc_class_depth(f: node, depth: number)
mc_class_depth(f, count : { mc_ancestor(f, c), mc_class(c, _) }) :- mc_function(f, _).
.decl mc_all_caps(name: symbol)
mc_all_caps(name) :- mc_assignment(_, _, name, _, _), contains("_", name), !prefix("_", name), !mc_upper_changes(name).
.decl mc_cache_like(name: symbol)
mc_cache_like(name) :- mc_assignment(_, _, name, _, _), mc_token(token), contains(token, @lower(name)).
.decl mc_constructor(n: node, name: symbol)
mc_constructor(n, name) :- mc_call(n, fn), mc_call_leaf(fn, name), mc_constructor_name(name).
mc_constructor(n, name) :- mc_call(n, fn), mc_call_leaf(fn, name), suffix("Cache", name).
.decl mc_managed(n: node)
mc_managed(n) :- mc_constructor(n, name), mc_managed_name(managed), match(cat("_*", managed), name), !mc_registration_disabled(n).
.decl mc_assignment_kind(n: node, name: symbol, pattern_type: symbol, severity: number, description: symbol)
mc_assignment_kind(n, name, "dict_cache", 3, cat("dict cache `", name, "`")) :- mc_assignment(_, _, name, n, _), mc_empty(n, "dict").
mc_assignment_kind(n, name, "set_cache", 2, cat("set cache `", name, "`")) :- mc_assignment(_, _, name, n, _), mc_empty(n, "set").
mc_assignment_kind(n, name, "list_cache", 2, cat("list cache `", name, "`")) :- mc_assignment(_, _, name, n, _), mc_empty(n, "list").
mc_assignment_kind(n, name, "list_cache", 2, cat("list-backed cache holder `", name, "`")) :- mc_assignment(_, _, name, n, _), mc_list(n), !mc_empty(n, "list"), mc_cache_like(name).
mc_assignment_kind(n, name, "constructor_cache", 3, cat("`", constructor, "` cache `", name, "`")) :- mc_assignment(_, _, name, n, _), mc_constructor(n, constructor), !mc_managed(n).
.decl mc_issue(n: node, name: symbol, pattern_type: symbol, severity: number, message: symbol, suborder: number)
mc_issue(a, name, pattern, severity, cat("Module-level ", description, " -- use `data_layer.cache` or add an explicit local-only exception"), sub) :-
 mc_assignment(a, _, name, v, sub), !mc_in_frame(a), name != "__all__", !mc_all_caps(name),
 mc_position(a, d, _, _, _), mc_path(d, path), !prefix("impl/common/l1000_cache/", path), !prefix("l1000_cache/", path),
 mc_assignment_kind(v, name, pattern, severity, description).
mc_issue(f, name, "lru_cache", 3, cat("Module-level `@", decorator, "` on `", name, "()` -- use `data_layer.cache` or add an explicit local-only exception"), sub) :-
 mc_function(f, name), !mc_outer_function(f), mc_class_depth(f, 0), mc_decorator(f, d, sub), mc_cache_decorator(d, decorator).
mc_issue(f, cat(cls, ".", name), "lru_cache", 2, cat("`@", decorator, "` on `", cls, ".", name, "()` -- use `data_layer.cache` or add an explicit local-only exception"), sub) :-
 mc_function(f, name), !mc_outer_function(f), mc_class_depth(f, 1), mc_ancestor(f, c), mc_class(c, cls), mc_decorator(f, d, sub), mc_cache_decorator(d, decorator).
'''


def outputs():
    return r'''
.decl mc_severity_name(severity: number, name: symbol)
mc_severity_name(1, "INFO"). mc_severity_name(2, "WARNING"). mc_severity_name(3, "CRITICAL").
.violation MCCount(at: span, severity: number, name: symbol, value: number) code "MC_SEVERITY_COUNT" "{name}: {value}"
MCCount(@place("", 0), severity, name, count : { option_item("severityValues", i, value), value = to_string(severity) }) :- option("entryPoint", "_counts_by_severity"), mc_severity_name(severity, name).
.decl mc_result(n: node, path: symbol, line: number, column: number, name: symbol, pattern_type: symbol, severity: number, message: symbol, visit: number, suborder: number, key: symbol, formatted: symbol)
mc_result(n, path, l, c, name, pattern, severity, message, i, sub, key, formatted) :- mc_issue(n, name, pattern, severity, message, sub),
 mc_position(n, d, l, c, i), mc_path(d, path), mc_selected(d), mc_severity_name(severity, sev),
 key = cat(path, "::", pattern, "::", name), formatted = cat(path, ":", to_string(l), ":", to_string(c), " [", sev, "] ", message).
.violation MCFact(at: span, column: number, name: symbol, pattern_type: symbol, severity: number, visit: number, suborder: number, key: symbol, formatted: symbol, value: symbol) code "MC_FACT" "{value}"
MCFact(@place(path, l), c, name, pattern, severity, i, sub, key, formatted, message) :- option("entryPoint", _), mc_result(_, path, l, c, name, pattern, severity, message, i, sub, key, formatted).
'''


def allowlist():
    text = r'''
.decl mc_al_entry(i: number, path: symbol, pattern: symbol, name: symbol, classification: symbol, reason: symbol, key: symbol, signature: symbol)
mc_al_entry(i, path, pattern, name, classification, reason, key, cat(key, "::", classification, "::", reason)) :-
 mc_al_path(i, path), mc_al_pattern(i, pattern), mc_al_name(i, name), mc_al_classification(i, classification), mc_al_reason(i, reason),
 key = cat(path, "::", pattern, "::", name).
.decl mc_al_display(value: symbol)
mc_al_display(value) :- option("allowlistDisplayPath", value).
mc_al_display("scripts/allowlists/lint_module_level_caches_allowlist.json") :- !option("allowlistDisplayPath", _).
.decl mc_al_error(i: number, rank: number, kind: symbol, value: symbol)
mc_al_error(-3, 0, "ValueError", detail) :- input_error("scripts/allowlists/lint_module_level_caches_allowlist.json", kind, detail), kind != "missing".
mc_al_error(-2, 0, "ValueError", cat(path, " must contain a JSON object")) :- mc_al_root_type(kind), kind != "object", mc_al_display(path).
mc_al_error(-1, 0, "ValueError", cat(path, " must contain an 'entries' list")) :- mc_al_entries_type(kind), kind != "array", mc_al_display(path).
mc_al_error(i, 0, "ValueError", "Allowlist entries must be objects") :- mc_al_item_type(i, kind), kind != "object".
'''
    for rank, (relation, key) in enumerate([('mc_al_path','path'), ('mc_al_pattern','pattern_type'),
            ('mc_al_name','name'), ('mc_al_classification','classification'), ('mc_al_reason','reason')],1):
        text += f'mc_al_error(i, {rank}, "KeyError", {json.dumps(key)}) :- mc_al_item_type(i, "object"), !{relation}(i, _).\n'
    text += r'''
mc_al_error(i, 6, "ValueError", cat("Allowlist entry missing classification: ", key)) :- mc_al_entry(i, _, _, _, _, _, key, _), mc_al_blank_classification(i).
mc_al_error(i, 7, "ValueError", cat("Allowlist entry missing reason: ", key)) :- mc_al_entry(i, _, _, _, _, _, key, _), mc_al_blank_reason(i).
mc_al_error(i, 8, "ValueError", cat("Duplicate allowlist entry: ", key)) :- mc_al_entry(i, _, _, _, _, _, key, _), mc_al_entry(j, _, _, _, _, _, key, _), j < i.
.decl mc_al_failed()
mc_al_failed() :- mc_al_error(_, _, _, _).
.violation MCAllowEntry(at: span, i: number, path: symbol, pattern_type: symbol, name: symbol, classification: symbol, reason: symbol, key: symbol, signature: symbol) code "MC_ALLOW_ENTRY" "{key}"
MCAllowEntry(@place("", 0), i, path, pattern, name, classification, reason, key, signature) :- option("entryPoint", "_load_allowlist_entries"), !mc_al_failed(), mc_al_entry(i, path, pattern, name, classification, reason, key, signature).
.violation MCAllowError(at: span, kind: symbol, value: symbol) code "MC_ALLOW_ERROR" "{value}"
MCAllowError(@place("", 0), kind, value) :- option("entryPoint", "_load_allowlist_entries"), mc_al_error(i, rank, kind, value),
 i = min j : { mc_al_error(j, _, _, _) }, rank = min r : { mc_al_error(i, r, _, _) }.
.decl mc_allowlisted(key: symbol)
mc_allowlisted(key) :- mc_al_entry(_, _, _, _, _, _, key, _).
'''
    for code in ['dict_cache', 'set_cache', 'list_cache', 'constructor_cache', 'lru_cache']:
        name = 'MCCache'+''.join(w.title() for w in code.split('_'))
        text += f'.violation {name}(at: span, column: number, name: symbol, visit: number, suborder: number, value: symbol) code "{code}" "{{value}}"\n'
        text += f'{name}(@place(path, l), c, name, i, sub, message) :- !option("entryPoint", _), mc_result(_, path, l, c, name, "{code}", _, message, i, sub, key, _), !mc_al_failed(), !mc_allowlisted(key).\n'
    text += r'''
.violation MCInvalidAllowlist(at: span, value: symbol) code "invalid_allowlist" "{value}"
MCInvalidAllowlist(@place("scripts/allowlists/lint_module_level_caches_allowlist.json", 1), value) :- !option("entryPoint", _), mc_al_error(i, rank, kind, value), kind != "KeyError",
 i = min j : { mc_al_error(j, _, _, _) }, rank = min r : { mc_al_error(i, r, _, _) }.
.violation MCUncaughtAllowlist(at: span, kind: symbol, value: symbol) code "MC_UNCAUGHT_CONFIG" "{value}"
MCUncaughtAllowlist(@place("scripts/allowlists/lint_module_level_caches_allowlist.json", 1), "KeyError", value) :- !option("entryPoint", _), mc_al_error(i, rank, "KeyError", value),
 i = min j : { mc_al_error(j, _, _, _) }, rank = min r : { mc_al_error(i, r, _, _) }.
'''
    return text
