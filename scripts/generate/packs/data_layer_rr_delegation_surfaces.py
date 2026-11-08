"""Shared source predicates for callback, transaction, and execution-lazy surfaces."""


def surface_relations():
    return r'''
.decl asds_return(f: node, text: symbol)
asds_return(f, text) :- asdp_function(f, _, _), field(f, "return_type", a), annotation_text(a, text).
.decl asds_lazy_decorated(f: node)
asds_lazy_decorated(f) :- asdp_decorator(f, _, name), (name = "lazy" ; name = "pure_lazy").
.decl asds_grammar_lazy(f: node)
asds_grammar_lazy(f) :- asdp_function(f, _, name), suffix("_lazy", name), asds_return(f, ann), contains("LazyResult", ann), asda_desc_call(f, call), field(call, "function", raw), unparen(raw, fn), asdp_callable(fn, helper), (helper = "grammar_effect_lazy" ; helper = "grammar_row_lazy" ; helper = "grammar_rows_lazy" ; helper = "grammar_value_lazy").
.decl asds_execution_lazy(f: node)
asds_execution_lazy(f) :- asds_grammar_lazy(f).
asds_execution_lazy(f) :- asds_lazy_decorated(f), !field(f, "return_type", _).
asds_execution_lazy(f) :- asds_lazy_decorated(f), asds_return(f, ann), (contains("LazyResult", ann) ; contains("LazySeq", ann) ; contains("ServerPlan", ann)).
.decl asds_parameter(f: node, nm: node, annotation: loc)
asds_parameter(f, nm, annotation) :- asdp_function(f, _, _), sig_param(f, _, nm, _, k, annotation, _), (k = "posonly" ; k = "pos" ; k = "kwonly").
.decl asds_callback(f: node)
asds_callback(f) :- asds_parameter(f, nm, _), text(nm, raw), name = @nfkc(raw), (name = "fn" ; name = "callback").
asds_callback(f) :- asds_parameter(f, _, ann), annotation_text(ann, text), contains("Callable", text).
.decl asds_transaction(f: node)
asds_transaction(f) :- asds_parameter(f, nm, _), !asds_callback(f), text(nm, raw), name = @nfkc(raw), (name = "txn" ; name = "tx" ; name = "vtxn" ; name = "read_txn" ; name = "write_txn" ; name = "base_txn").
asds_transaction(f) :- asds_parameter(f, _, ann), !asds_callback(f), annotation_text(ann, text), contains("Transaction", text).
.decl asds_unsupported(f: node)
asds_unsupported(f) :- asda_desc_call(f, call), field(call, "function", raw), unparen(raw, fn), kind(fn, "attribute"), field(fn, "attribute", attr), text(attr, name), @nfkc(name) = "_raise_async_not_supported".
'''
