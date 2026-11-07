"""Port connection/transaction annotation traversal, precedence, reporting, and signature gating."""
import json,re
from data_layer_rr_text import module,classes,character_class


def conn_or_txn_union(source,target,header):
    original=module(source,'conn_or_txn_union');classes();q=json.dumps
    text=header('conn_or_txn_union',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "impl/**/*.py", "conform/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l*/**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude "**/tests/**", "**/test/**", "**/.git/**", "**/.venv/**", "**/venv/**", "**/__pycache__/**", "**/tmp/**"',text,flags=re.M)
    text+='.severity ConnectionUnion error\n.severity Separate error\n.severity InvalidSignature error\n#include "python/print.rr"\n#include "data-layer/signing.rr"\n'
    for relation,values in [('cut_connection',original.CONNECTION_TYPES),('cut_transaction',original.TRANSACTION_TYPES),('cut_allow',original.ALLOWLIST)]:
        text+=f'.decl {relation}(name: symbol)\n'+''.join(f'{relation}({q(v)}).\n' for v in sorted(values))
    text+=''.join(f'allow_entry("conn_or_txn_union", {q(v)}).\n' for v in sorted(original.ALLOWLIST))
    text+=f'allow_sig("conn_or_txn_union", "lint_conn_or_txn_union", {q(original._ALLOWLIST_SIGNATURE)}).\n'
    text+='.decl cut_layer(d: number)\ncut_layer(d) :- doc(d, p, _), match('+q('(?s)l'+character_class(str.isdecimal)+r'{4}_[^/\n][^/]*/.*')+', p).\n'
    return text+r'''
.decl cut_test(d: number)
cut_test(d) :- path_part(d, "tests").
cut_test(d) :- path_part(d, "test").
.decl cut_hidden(d: number)
cut_hidden(d) :- path_part(d, x), (x = ".git" ; x = ".venv" ; x = "venv" ; x = "__pycache__" ; x = "tmp").
.decl cut_trigger(d: number)
cut_trigger(d) :- source(d, sp), text(sp, t), cut_transaction(name), contains(name, t).
.decl cut_document(d: number)
cut_document(d) :- doc(d, _, _), file_name(d, name), suffix(".py", name), !cut_test(d), cut_trigger(d),
    !syntax_error(d, _, _), option("entryPoint", "analyze_file").
cut_document(d) :- doc(d, p, _), file_name(d, name), suffix(".py", name), !prefix("test_", name),
    !cut_test(d), !cut_hidden(d), cut_trigger(d), !syntax_error(d, _, _), !option("entryPoint", _),
    (prefix("impl/", p) ; prefix("conform/", p) ; prefix("data_layer/", p) ; prefix("_internal/", p) ; cut_layer(d)).
.decl cut_class_kind(kind: symbol)
cut_class_kind("class_definition").
.decl cut_qualified(f: node, bare: symbol, qualified: symbol)
cut_qualified(f, @nfkc(name), cat(@nfkc(cls), ".", @nfkc(name))) :- kind(f, "function_definition"), def_name(f, name),
    nearest(f, c, cut_class_kind), def_name(c, cls).
cut_qualified(f, @nfkc(name), @nfkc(name)) :- kind(f, "function_definition"), def_name(f, name), !nearest(f, _, cut_class_kind).
.decl cut_allowed(f: node)
cut_allowed(f) :- cut_qualified(f, name, q), (cut_allow(name) ; cut_allow(q)).
.decl cut_rank(kind: symbol, rank: number)
cut_rank("pos", 0). cut_rank("posonly", 1). cut_rank("kwonly", 2).
.decl cut_parameter(f: node, p: node, name: symbol, ann: node, rank: number, index: number)
cut_parameter(f, p, @nfkc(name), ann, rank, i) :- kind(f, "function_definition"), docof(f, d), cut_document(d),
    !cut_allowed(f), sig_param(f, p, nm, i, kind, ann, _), cut_rank(kind, rank), !unset(ann), text(nm, name), @nfkc(name) != "self".
// Follow precisely _get_all_type_names: attributes stop at their final name; only BitOr binary nodes recurse;
// tuples recurse only as subscript slices. A quoted string is one entire name and is never parsed as a type.
.decl cut_edge(e: node, c: node)
cut_edge(e, c) :- kind(e, "type"), child(e, c, 0).
cut_edge(e, c) :- kind(e, "parenthesized_expression"), child(e, c, _), !kind(c, "comment").
cut_edge(e, c) :- kind(e, "generic_type"), child(e, c, 0).
cut_edge(e, c) :- kind(e, "subscript"), field(e, "value", c).
// A parenthesized tuple slice is still a tuple in Python's AST. Strip wrappers before classifying it.
.decl cut_slice(e: node, slice: node)
cut_slice(e, s) :- kind(e, "generic_type"), child(e, tp, 1), kind(tp, "type_parameter"), child(tp, s, _).
cut_slice(e, s) :- kind(e, "subscript"), field(e, "subscript", s).
cut_slice(e, c) :- cut_slice(e, s), (kind(s, "type") ; kind(s, "parenthesized_expression")), child(s, c, _), !kind(c, "comment").
cut_edge(e, s) :- cut_slice(e, s), !kind(s, "tuple").
cut_edge(e, c) :- cut_slice(e, t), kind(t, "tuple"), child(t, c, _).
cut_edge(e, c) :- kind(e, "binary_operator"), op(e, "|"), (field(e, "left", c) ; field(e, "right", c)).
cut_edge(e, c) :- kind(e, "union_type"), child(e, c, _).
.decl cut_annotation(e: node)
cut_annotation(e) :- cut_parameter(_, _, _, e, _, _).
cut_annotation(c) :- cut_annotation(e), cut_edge(e, c).
.decl cut_name(e: node, name: symbol)
cut_name(e, @nfkc(name)) :- cut_annotation(e), ast_name(e, name).
cut_name(e, @nfkc(name)) :- cut_annotation(e), kind(e, "attribute"), field(e, "attribute", a), text(a, name).
cut_name(e, name) :- cut_annotation(e), plain_str(e, name).
cut_name(e, name) :- cut_annotation(e), cut_edge(e, c), cut_name(c, name).
.decl cut_conn(e: node)
cut_conn(e) :- cut_name(e, name), cut_connection(name).
.decl cut_txn(e: node)
cut_txn(e) :- cut_name(e, name), cut_transaction(name).
.decl cut_union(f: node, p: node, name: symbol, ann: node, rank: number, index: number)
cut_union(f, p, name, ann, r, i) :- cut_parameter(f, p, name, ann, r, i), cut_conn(ann), cut_txn(ann).
.decl cut_first_union(f: node, p: node, name: symbol, ann: node)
cut_first_union(f, p, name, ann) :- cut_union(f, p, name, ann, r, i),
    r = min rank : { cut_union(f, _, _, _, rank, _) }, i = min index : { cut_union(f, _, _, _, r, index) }.
.decl cut_issue(f: node, kind: symbol, details: symbol)
cut_issue(f, "UNION", cat("Parameter '", name, "' accepts both: ", printed)) :-
    cut_first_union(f, _, name, ann), annotation_text(ann, printed).
cut_issue(f, "SEPARATE_PARAMS", cat("Has both connection (", connections, ") and transaction (", transactions, ") params")) :-
    cut_parameter(f, _, _, a, _, _), cut_conn(a), cut_parameter(f, _, _, b, _, _), cut_txn(b), !cut_union(f, _, _, _, _, _),
    connections = join item by (r, i) sep ", " : { cut_parameter(f, _, name, ann, r, i), cut_conn(ann), annotation_text(ann, printed), item = cat(name, ": ", printed) },
    transactions = join item by (r, i) sep ", " : { cut_parameter(f, _, name, ann, r, i), cut_txn(ann), annotation_text(ann, printed), item = cat(name, ": ", printed) }.
.violation UnionAPI(at: node, function_name: symbol, details: symbol, ordinal: number) code "UNION" "{details}"
UnionAPI(f, q, details, i) :- cut_issue(f, "UNION", details), cut_qualified(f, _, q), visit_index(f, i), option("entryPoint", "analyze_file").
.violation SeparateAPI(at: node, function_name: symbol, details: symbol, ordinal: number) code "SEPARATE_PARAMS" "{details}"
SeparateAPI(f, q, details, i) :- cut_issue(f, "SEPARATE_PARAMS", details), cut_qualified(f, _, q), visit_index(f, i), option("entryPoint", "analyze_file").
.violation ConnectionUnion(at: node, function_name: symbol, details: symbol, ordinal: number) code "UNION" "{function_name}: {details}"
ConnectionUnion(f, q, details, i) :- cut_issue(f, "UNION", details), cut_qualified(f, _, q), visit_index(f, i), !option("entryPoint", _), allow_status("conn_or_txn_union", "valid").
.violation Separate(at: node, function_name: symbol, details: symbol, ordinal: number) code "SEPARATE_PARAMS" "{function_name}: {details}"
Separate(f, q, details, i) :- cut_issue(f, "SEPARATE_PARAMS", details), cut_qualified(f, _, q), visit_index(f, i), !option("entryPoint", _), allow_status("conn_or_txn_union", "valid").
.violation DecodeFailure(at: span) code "CUT_DECODE_ERROR" "Source decoding failed"
DecodeFailure(@place("input.py", 1)) :- option("entryPoint", "analyze_file"), option("decodeError", "true").
.violation InvalidSignature(at: span) code "unsigned_allowlist" "connection/transaction allowlist signature verification failed"
InvalidSignature(@place("scripts/lint_conn_or_txn_union.py", 1)) :- !option("entryPoint", _), !allow_status("conn_or_txn_union", "valid").
'''
