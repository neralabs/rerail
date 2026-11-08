"""Native transaction-parameter detection and its ordered public helper contracts."""
import json,re
from data_layer_rr_text import module,character_class
from data_layer_rr_pattern_categories import inference


def transaction_patterns(source,target,header):
    original=module(source,'transaction_patterns');q=json.dumps
    text=header('transaction_patterns',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    for rel,values in [('tp_core',original.CORE_LEGITIMATE_FUNCTIONS),('tp_init_pattern',original.ALLOWED_INIT_CLASS_PATTERNS),
        ('tp_bridge_pattern',original.TS_BRIDGE_MODULE_PATTERNS),('tp_trigger',original._TRANSACTION_PATTERN_SOURCE_TRIGGERS),
        ('tp_failing',original._FAILING_CATEGORIES),('tp_internal_allowed',{'LEGITIMATE','BOOTSTRAP','BACKGROUND','BACKEND_OPS','VALIDATION','ADMIN_TOOLS'})]:
        text+=f'.decl {rel}(value: symbol)\n'+''.join(f'{rel}({q(x)}).\n' for x in sorted(values))
    categories=list(original._get_categorized_functions(None))
    text+='.decl tp_default_key(i: number, category: symbol)\n'+''.join(f'tp_default_key({i}, {q(c)}).\n' for i,c in enumerate(categories))
    text+='.decl tp_layer(path: symbol)\ntp_layer(path) :- doc(_, path, _), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', path).\n'
    text+=projection()+inference()+categories_and_helpers()+detector()
    text+=f'.decl tp_public_key(value: symbol)\ntp_public_key({q(original.BASELINE_PUBLIC_KEY_HEX)}).\n'
    text+=f'.decl tp_baseline_file(value: symbol)\ntp_baseline_file({q(original.BASELINE_FILE)}).\n'
    return text


def projection():
    return r'''
.decl tp_graph()
tp_graph() :- option_item("astKinds", 0, _).
.decl tp_gkind(n: node, kind: symbol)
tp_gkind(n, kind) :- option_item("astKinds", n, kind).
.decl tp_edge(parent: node, field: symbol, child: node)
tp_edge(to_number(p), field, c) :- option_item("astParents", c, p), p != "-1", option_item("astFields", c, field).
.decl tp_ancestor(n: node, ancestor: node)
tp_ancestor(n, p) :- tp_edge(p, _, n).
tp_ancestor(n, a) :- tp_edge(p, _, n), tp_ancestor(p, a).
.decl tp_function(f: node, name: symbol)
tp_function(f, @nfkc(name)) :- !tp_graph(), kind(f, "function_definition"), def_name(f, name).
tp_function(f, name) :- tp_gkind(f, k), (k = "FunctionDef" ; k = "AsyncFunctionDef"), option_item("astNames", f, name).
.decl tp_doc(d: number, path: symbol)
tp_doc(d, path) :- !tp_graph(), doc(d, physical, _), doc_role(d, "primary"), option_item("documentPaths", i, physical), option_item("originalPaths", i, path).
tp_doc(d, path) :- !tp_graph(), doc(d, path, _), doc_role(d, "primary"), !option_item("documentPaths", _, path).
tp_doc(0, path) :- tp_graph(), option("originalPath", path).
.decl tp_position(f: node, d: number, line: number, entry_order: number, exit_order: number)
tp_position(f, d, l, i, e) :- !tp_graph(), tp_function(f, _), node(d, f, _, _, e), line(f, l), visit_index(f, i).
tp_position(f, 0, to_number(l), f, to_number(e)) :- tp_function(f, _), tp_gkind(f, _), option_item("astLines", f, l), option_item("astExitOrder", f, e).
.decl tp_class(c: node, name: symbol)
tp_class(c, @nfkc(name)) :- !tp_graph(), kind(c, "class_definition"), def_name(c, name).
tp_class(c, name) :- tp_gkind(c, "ClassDef"), option_item("astNames", c, name).
.decl tp_current_class(f: node, name: symbol)
tp_current_class(f, @nfkc(name)) :- !tp_graph(), nearest(f, c, "class_definition"), def_name(c, name).
tp_current_class(f, name) :- tp_graph(), tp_ancestor(f, c), tp_class(c, name), c = max a : { tp_ancestor(f, a), tp_class(a, _) }.
.decl tp_owner(n: node, f: node)
tp_owner(n, f) :- !tp_graph(), def_frame(n, f).
tp_owner(n, f) :- !tp_graph(), nearest(n, dc, def_mark), kind(dc, "decorator"), child(dd, dc, _),
    field(dd, "definition", c), kind(c, "class_definition"), tp_owner(c, f).
tp_owner(n, f) :- tp_graph(), tp_ancestor(n, f), tp_function(f, _), f = max a : { tp_ancestor(n, a), tp_function(a, _) }.
.decl tp_name(n: node, name: symbol)
tp_name(n, @nfkc(name)) :- !tp_graph(), unparen(n, p), ast_name(p, name).
tp_name(n, name) :- tp_gkind(n, "Name"), option_item("astIds", n, name).
.decl tp_attribute(n: node, object: node, name: symbol)
tp_attribute(n, o, @nfkc(name)) :- !tp_graph(), kind(n, "attribute"), field(n, "object", raw), unparen(raw, o), field(n, "attribute", a), text(a, name).
tp_attribute(n, o, name) :- tp_gkind(n, "Attribute"), tp_edge(n, "value", o), option_item("astAttrs", n, name).
.decl tp_call(c: node, fn: node, i: number)
tp_call(c, fn, i) :- !tp_graph(), kind(c, "call"), field(c, "function", raw), unparen(raw, fn), visit_index(c, i).
tp_call(c, fn, c) :- tp_gkind(c, "Call"), tp_edge(c, "func", fn).
.decl tp_annotation_edge(n: node, c: node)
tp_annotation_edge(n, c) :- !tp_graph(), (kind(n, "type") ; kind(n, "parenthesized_expression")), child(n, c, _), !kind(c, "comment").
tp_annotation_edge(n, c) :- !tp_graph(), kind(n, "subscript"), field(n, "value", raw), unparen(raw, c).
tp_annotation_edge(n, c) :- !tp_graph(), kind(n, "generic_type"), child(n, c, 0).
tp_annotation_edge(n, c) :- tp_gkind(n, "Subscript"), tp_edge(n, "value", c).
.decl tp_binary(n: node, left: node, right: node)
tp_binary(n, l, r) :- !tp_graph(), kind(n, "binary_operator"), field(n, "left", l), field(n, "right", r).
tp_binary(n, l, r) :- !tp_graph(), kind(n, "union_type"), child(n, l, 0), child(n, r, 1).
tp_binary(n, l, r) :- tp_gkind(n, "BinOp"), tp_edge(n, "left", l), tp_edge(n, "right", r).
.decl tp_annotation(n: node, head: symbol)
tp_annotation(n, name) :- tp_name(n, name).
tp_annotation(n, name) :- tp_attribute(n, _, name).
tp_annotation(n, value) :- !tp_graph(), plain_str(n, value).
tp_annotation(n, cat("b'", value, "'")) :- !tp_graph(), string_form(n, "bytes"), value(n, value).
tp_annotation(n, value) :- tp_gkind(n, "Constant"), option_item("astLiteralText", n, value).
tp_annotation(n, "") :- tp_gkind(n, k), k != "Name", k != "Attribute", k != "Constant", k != "Subscript", k != "BinOp".
tp_annotation(n, "") :- !tp_graph(), kind(n, k), k != "identifier", k != "attribute", k != "string", k != "concatenated_string",
    k != "type", k != "parenthesized_expression", k != "subscript", k != "generic_type", k != "union_type", k != "binary_operator".
tp_annotation(n, value) :- tp_annotation_edge(n, c), tp_annotation(c, value).
tp_annotation(n, value) :- tp_binary(n, l, _), tp_annotation(l, value), contains("Connection", value).
tp_annotation(n, value) :- tp_binary(n, l, r), tp_annotation(l, left), !contains("Connection", left), tp_annotation(r, value), contains("Connection", value).
tp_annotation(n, "") :- tp_binary(n, l, r), tp_annotation(l, left), !contains("Connection", left), tp_annotation(r, right), !contains("Connection", right).
.decl tp_connection_annotation(n: node)
tp_connection_annotation(n) :- tp_annotation(n, name), name != "ConnectionId", (name = "conn" ; contains("Connection", name)).
.decl tp_parameter(f: node, name: symbol, annotation: node, i: number)
tp_parameter(f, @nfkc(name), ann, i) :- !tp_graph(), sig_param(f, _, nm, i, "pos", ann, _), !unset(ann), text(nm, name), @nfkc(name) != "self".
tp_parameter(f, name, ann, to_number(i)) :- tp_function(f, _), tp_edge(f, "args", a), tp_edge(a, "args", p),
    option_item("astArgNames", p, name), name != "self", tp_edge(p, "annotation", ann), option_item("astOrdinals", p, i).
.decl tp_first_parameter(f: node, name: symbol)
tp_first_parameter(f, name) :- tp_parameter(f, name, ann, i), tp_connection_annotation(ann),
    i = min j : { tp_parameter(f, _, a, j), tp_connection_annotation(a) }.
.decl tp_internal_call(c: node, connection: symbol, method: symbol, i: number)
tp_internal_call(c, name, method, i) :- tp_call(c, fn, i), tp_attribute(fn, receiver, method), tp_name(receiver, name),
    (method = "write" ; method = "read" ; method = "schema_transaction").
'''


def categories_and_helpers():
    return r'''
.decl tp_category_key(i: number, category: symbol)
tp_category_key(i, category) :- option_item("categoryKeys", i, category), !option("mergeCategories", "true").
tp_category_key(i, category) :- tp_default_key(i, category), (option("mergeCategories", "true") ; !option("categoriesProvided", _)).
tp_category_key(i + 12, category) :- option_item("categoryKeys", i, category), option("mergeCategories", "true"), !tp_default_key(_, category).
.decl tp_category_member(i: number, category: symbol, name: symbol)
tp_category_member(i, category, name) :- tp_category_key(i, category), option_item("categoryMemberKeys", j, category), option_item("categoryMemberNames", j, name).
tp_category_member(i, "LEGITIMATE", name) :- tp_category_key(i, "LEGITIMATE"), tp_core(name), (option("mergeCategories", "true") ; !option("categoriesProvided", _)).
.decl tp_is_init(name: symbol, cls: symbol)
tp_is_init("__init__", cls) :- tp_current_class(_, cls), tp_init_pattern(pattern), contains(pattern, cls).
tp_is_init("__init__", cls) :- option("className", cls), tp_init_pattern(pattern), contains(pattern, cls).
.decl tp_base_category(f: node, category: symbol)
tp_base_category(f, "LEGITIMATE") :- tp_function(f, name), tp_current_class(f, cls), tp_is_init(name, cls).
tp_base_category(f, category) :- tp_function(f, name), !tp_init_function(f), tp_category_member(i, category, name),
    i = min j : { tp_category_member(j, _, name) }.
tp_base_category(f, "TS_BRIDGE") :- tp_function(f, name), !tp_init_function(f), !tp_category_member(_, _, name),
    tp_position(f, d, _, _, _), tp_doc(d, path), tp_bridge_pattern(pattern), contains(pattern, path).
tp_base_category(f, category) :- tp_function(f, name), !tp_init_function(f), !tp_category_member(_, _, name),
    !tp_in_bridge(f), tp_position(f, d, _, _, _), tp_doc(d, path), tp_inferred(name, path, category).
.decl tp_init_function(f: node)
tp_init_function(f) :- tp_function(f, name), tp_current_class(f, cls), tp_is_init(name, cls).
.decl tp_in_bridge(f: node)
tp_in_bridge(f) :- tp_position(f, d, _, _, _), tp_doc(d, path), tp_bridge_pattern(pattern), contains(pattern, path).
.violation TPCategoryKey(at: span, i: number, category: symbol) code "TP_CATEGORY_KEY" "{category}"
TPCategoryKey(@place("", 0), i, category) :- option("entryPoint", "_get_categorized_functions"), tp_category_key(i, category).
.violation TPCategoryMember(at: span, i: number, category: symbol, name: symbol) code "TP_CATEGORY_MEMBER" "{name}"
TPCategoryMember(@place("", 0), i, category, name) :- option("entryPoint", "_get_categorized_functions"), tp_category_member(i, category, name).
.violation TPInit(at: span, answer: number) code "TP_BOOL" "{answer}"
TPInit(@place("", 0), 1) :- option("entryPoint", "_is_connection_class_init"), option("functionName", name), option("className", cls), tp_is_init(name, cls).
TPInit(@place("", 0), 0) :- option("entryPoint", "_is_connection_class_init"), option("functionName", name), !tp_init_answer(name).
.decl tp_init_answer(name: symbol)
tp_init_answer(name) :- option("functionName", name), option("className", cls), tp_is_init(name, cls).
.decl tp_signature_part(category: symbol, value: symbol)
tp_signature_part(category, item) :- option_item("signatureKeys", _, category),
    body = join quoted by (value, i) sep "," : { option_item("signatureValueKeys", i, category), option_item("signatureValues", i, value), quoted = @json_str(value) },
    item = cat(@json_str(category), ":[", body, "]").
.decl tp_signature_message(value: symbol)
tp_signature_message(cat("{", body, "}")) :- option("signatureInput", "true"),
    body = join item by category sep "," : { tp_signature_part(category, item) }.
.violation TPSignature(at: span, value: symbol) code "TP_SIGNATURE_MESSAGE" "{value}"
TPSignature(@place("", 0), value) :- option("entryPoint", "_compute_signature_message"), tp_signature_message(value).
.violation TPVerify(at: span, answer: number) code "TP_BOOL" "{answer}"
TPVerify(@place("", 0), @ed25519_ok(key, message, signature)) :- option("entryPoint", "_verify_signature"),
    option("publicKey", key), option("signature", signature), tp_signature_message(message).
.violation TPInternal(at: span, method: symbol, visit: number) code "TP_INTERNAL" "{method}"
TPInternal(@place("", 0), method, i) :- option("internalProbe", "true"), option("connectionName", name), tp_internal_call(_, name, method, i).
.violation TPConstant(at: span, kind: symbol, value: symbol) code "TP_CONSTANT" "{value}"
TPConstant(@place("", 0), "core", value) :- option("entryPoint", "constants"), tp_core(value).
TPConstant(@place("", 0), "init", value) :- option("entryPoint", "constants"), tp_init_pattern(value).
TPConstant(@place("", 0), "bridge", value) :- option("entryPoint", "constants"), tp_bridge_pattern(value).
TPConstant(@place("", 0), "key", value) :- option("entryPoint", "constants"), tp_public_key(value).
TPConstant(@place("", 0), "file", value) :- option("entryPoint", "constants"), tp_baseline_file(value).
'''


def detector():
    text=r'''
.decl tp_triggered(d: number)
tp_triggered(d) :- doc(d, _, _), source(d, sp), text(sp, source), tp_trigger(trigger), contains(trigger, source).
.decl tp_selected(d: number)
tp_selected(0) :- tp_graph().
tp_selected(d) :- !tp_graph(), tp_doc(d, _), option("entryPoint", _), tp_triggered(d), !syntax_error(d, _, _), !tp_skipped(d).
tp_selected(d) :- !tp_graph(), tp_doc(d, p), !option("entryPoint", _), tp_triggered(d), !syntax_error(d, _, _), !tp_skipped(d),
    (prefix("impl/", p) ; prefix("data_layer/", p) ; prefix("_internal/", p) ; tp_layer(p)).
.decl tp_skipped(d: number)
tp_skipped(d) :- doc(d, physical, _), !option_item("documentPaths", _, physical), path_part(d, p), (p = "test" ; p = "tests").
tp_skipped(d) :- doc(d, physical, _), option_item("documentPaths", i, physical), option_item("scopePaths", i, scope), match("(?s)(?:.*/)?(?:tests|test)(?:/.*)?", scope).
.decl tp_owned_call(f: node, method: symbol, visit: number)
tp_owned_call(f, method, i) :- tp_first_parameter(f, name), tp_internal_call(c, name, method, i), tp_owner(c, f).
.decl tp_final_category(f: node, category: symbol)
tp_final_category(f, "INTERNAL_TXN_VIOLATION") :- tp_base_category(f, category), !tp_internal_allowed(category), tp_owned_call(f, _, _).
tp_final_category(f, category) :- tp_base_category(f, category), tp_internal_allowed(category).
tp_final_category(f, category) :- tp_base_category(f, category), !tp_owned_call(f, _, _).
.decl tp_finding(f: node, param: symbol, category: symbol, calls: symbol)
tp_finding(f, param, category, calls) :- tp_first_parameter(f, param), tp_final_category(f, category),
    tp_position(f, d, _, _, _), tp_selected(d),
    calls = join method by i sep "," : { tp_owned_call(f, method, i) }.
.violation TPAnalysis(at: span, name: symbol, param: symbol, category: symbol, calls: symbol, entry_order: number, exit_order: number) code "TP_FINDING" "{category}"
TPAnalysis(@place(path, l), name, param, category, calls, entry, exit) :- tp_finding(f, param, category, calls),
    tp_function(f, name), tp_position(f, d, l, entry, exit), tp_doc(d, path), option("entryPoint", _), !option("internalProbe", "true").
'''
    for category in ['NEEDS_REVIEW','TS_BRIDGE','INTERNAL_TXN_VIOLATION']:
        text+=f'''
.violation TPFinding{category}(at: span, name: symbol, param: symbol, entry_order: number, exit_order: number) code "{category}" "{{name}}({{param}}) must be refactored to take a transaction"
TPFinding{category}(@place(path, l), name, param, entry, exit) :- tp_finding(f, param, "{category}", _),
    tp_function(f, name), tp_position(f, d, l, entry, exit), tp_doc(d, path), !option("entryPoint", _).
'''
    return text
