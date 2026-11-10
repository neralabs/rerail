"""Port the original mismatch predicate, including the collector's dictionary semantics."""
import json
import re
from data_layer_rr_text import module, classes, character_class


def sync_async_signatures(source, target, header):
    original = module(source, 'sync_async_signatures')
    classes()  # Pin the offline generator's Unicode version.
    digits = character_class(str.isdecimal)
    text = header('sync_async_signatures', target)
    text = text.replace('.mode log\n', '.mode block\n')
    text = re.sub(r'^\.scope include .*$', '.scope include "api/**/*.py", "impl/**/*.py", "data_layer/**/*.py", "_internal/**/*.py", "l*/**/*.py"', text, flags=re.M)
    from data_layer_rr_cli import api_scope
    text=api_scope(text)
    text += '.decl sas_transaction(t: symbol)\n'
    text += ''.join(f'sas_transaction({json.dumps(t)}).\n' for t in sorted(original.TRANSACTION_TYPES))
    text += '.decl sas_connection(t: symbol)\n'
    text += ''.join(f'sas_connection({json.dumps(t)}).\n' for t in sorted(original.CONNECTION_TYPES))
    text += '.decl sas_layer(p: symbol)\n'
    text += 'sas_layer(p) :- doc(_, p, _), match(' + json.dumps(r'l' + digits + r'{4}_[^/]+/.*') + ', p).\n'
    return text + r'''
.decl sas_interest(t: symbol)
sas_interest(t) :- sas_transaction(t).
sas_interest(t) :- sas_connection(t).
.decl sas_test(d: number)
sas_test(d) :- path_part(d, "tests").
sas_test(d) :- path_part(d, "test").
.decl sas_hidden(d: number)
sas_hidden(d) :- path_part(d, ".git").
sas_hidden(d) :- path_part(d, ".venv").
sas_hidden(d) :- path_part(d, "venv").
sas_hidden(d) :- path_part(d, "__pycache__").
sas_hidden(d) :- path_part(d, "tmp").
.decl sas_trigger(d: number)
sas_trigger(d) :- source(d, sp), text(sp, s),
    (contains("_async", s) ; contains("Transaction", s) ; contains("Connection", s)).
.decl sas_document(d: number)
// Explicit-file discovery requires .py but does not exclude hidden directories.
sas_document(d) :- doc(d, _, _), option("entryPoint", "analyze_file"), file_name(d, f), suffix(".py", f),
    !sas_test(d), sas_trigger(d), !syntax_error(d, _, _).
sas_document(d) :- doc(d, p, _), !option("entryPoint", _), file_name(d, f), suffix(".py", f),
    (prefix("api/", p) ; prefix("impl/", p) ; prefix("data_layer/", p) ; prefix("_internal/", p) ; sas_layer(p)),
    !sas_test(d), !sas_hidden(d), sas_trigger(d), !syntax_error(d, _, _).

// The collector reads args.args only, skips self, and includes methods and nested functions.
.decl sas_pos(f: node, p: node, i: number, ann: loc)
sas_pos(f, nm, i, ann) :- kind(f, "function_definition"), docof(f, d), sas_document(d),
    sig_param(f, _, nm, i, "pos", ann, _), text(nm, raw), @nfkc(raw) != "self".
.decl sas_parameter(f: node, p: node, ann: loc)
sas_parameter(f, p, ann) :- sas_pos(f, p, i, ann), i = min j : { sas_pos(f, _, j, _) }.
.decl sas_function(f: node, d: number, name: symbol, i: number)
sas_function(f, d, @nfkc(raw), i) :- sas_parameter(f, _, _), docof(f, d), def_name(f, raw), visit_index(f, i).
// Even an untyped signature overwrites a typed signature. A function with no eligible parameter does not.
.decl sas_last(d: number, name: symbol, f: node)
sas_last(d, name, f) :- sas_function(f, d, name, i), i = max j : { sas_function(_, d, name, j) }.
.decl sas_first(d: number, name: symbol, i: number)
sas_first(d, name, i) :- sas_function(_, d, name, _), i = min j : { sas_function(_, d, name, j) }.

// Exactly the annotation edges the source recurses through. A subscript inspects its head, not its slice;
// an attribute inspects its final name, not its receiver. Every binary operator inspects left then right.
.decl sas_annotation_edge(e: node, c: node)
sas_annotation_edge(e, c) :- kind(e, "type"), child(e, c, 0).
sas_annotation_edge(e, c) :- kind(e, "parenthesized_expression"), child(e, c, _), !kind(c, "comment").
sas_annotation_edge(e, c) :- kind(e, "generic_type"), child(e, c, 0).
sas_annotation_edge(e, c) :- kind(e, "subscript"), field(e, "value", c).
sas_annotation_edge(e, c) :- kind(e, "binary_operator"), field(e, "left", c).
sas_annotation_edge(e, c) :- kind(e, "binary_operator"), field(e, "right", c).
sas_annotation_edge(e, c) :- kind(e, "union_type"), child(e, c, _).
.decl sas_annotation(e: node)
sas_annotation(e) :- sas_parameter(_, _, e), !unset(e).
sas_annotation(c) :- sas_annotation(e), sas_annotation_edge(e, c).
.decl sas_type_leaf(e: node, t: symbol, leaf: node)
sas_type_leaf(e, @nfkc(raw), e) :- sas_annotation(e), kind(e, "identifier"), text(e, raw), sas_interest(@nfkc(raw)).
sas_type_leaf(e, @nfkc(raw), e) :- sas_annotation(e), kind(e, "attribute"), field(e, "attribute", a), text(a, raw), sas_interest(@nfkc(raw)).
sas_type_leaf(e, t, e) :- sas_annotation(e), plain_str(e, t), sas_interest(t).
sas_type_leaf(e, t, leaf) :- sas_annotation(e), sas_annotation_edge(e, c), sas_type_leaf(c, t, leaf).
// A mismatch needs an interesting type. Among such leaves the source always prefers the leftmost one.
// Non-interesting Constant values cannot produce a mismatch and do not need to be serialized.
.decl sas_type(e: node, t: symbol)
sas_type(e, t) :- sas_type_leaf(e, t, leaf), visit_index(leaf, i),
    i = min j : { sas_type_leaf(e, _, other), visit_index(other, j) }.
.decl sas_async(f: node, flag: number)
sas_async(f, 1) :- kind(f, "function_definition"), op(f, "async").
sas_async(f, 0) :- kind(f, "function_definition"), !op(f, "async").

.violation Signature(at: node, sync_name: symbol, async_name: symbol, sync_param: symbol, async_param: symbol,
    sync_type: symbol, async_type: symbol, sync_line: number, async_line: number,
    sync_is_async: number, async_is_async: number, ordinal: number) code "sync_async_signature_mismatch"
    "{async_name} should take {sync_type}, not {async_type}"
Signature(af, sn, an, sp, ap, st, at, sl, al, sa, aa, ordinal) :-
    sas_last(d, an, af), suffix("_async", an), sn = @replace_all(an, "_async$", ""), sas_last(d, sn, sf),
    sas_parameter(sf, spn, sann), sas_parameter(af, apn, aann),
    sas_type(sann, st), sas_transaction(st), sas_type(aann, at), sas_connection(at),
    text(spn, sraw), sp = @nfkc(sraw), text(apn, araw), ap = @nfkc(araw),
    line(sf, sl), line(af, al), sas_async(sf, sa), sas_async(af, aa), sas_first(d, an, ordinal).
'''
