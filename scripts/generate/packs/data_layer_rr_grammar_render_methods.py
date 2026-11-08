"""Translate grammar render contexts and findings into the current native rule language."""
import json
from data_layer_rr_text import module, character_class


def grammar_render_methods(source,target,header):
    original=module(source,'grammar_render_methods')
    text=header('grammar_render_methods',target)+'#include "python/print.rr"\n'
    text+='\n.decl grm_metadata(key: symbol, value: symbol)\n'
    for name in ['ALLOWLIST_PUBLIC_KEY_HEX','ALLOWLIST_SIGNATURE']:
        text+='grm_metadata('+json.dumps(name)+','+json.dumps(getattr(original,name))+').\n'
    text+='grm_metadata("autofix",'+json.dumps('\nMechanical cleanup:\n  uv run python scripts/codemod_hoist_render_raw_strings.py impl/common/l2100_sql_grammar\n  uv run python scripts/fix_grammar_linter_violations.py --write\n  uv run python scripts/rollup_sql_fragments.py --write\n')+').\n'
    space=character_class(str.isspace)
    text+='\n.decl grm_trim(raw: symbol, text: symbol)\ngrm_trim(raw, @replace_all(raw, '+json.dumps('^'+space+'+|'+space+'+$')+', "")) :- option_item("sourceLines", _, raw).\n'
    text+='grm_trim(raw, @replace_all(raw, '+json.dumps('^'+space+'+|'+space+'+$')+', "")) :- src_line(_, _, raw).\n'
    return text+metadata()+helpers()+production()


def metadata():
    return r'''
.violation GRMMetadata(at: span, key: symbol, value: symbol) code "GRM_METADATA" "{value}"
GRMMetadata(@place("", 0), k, v) :- option("entryPoint", "configuration"), grm_metadata(k, v).
GRMMetadata(@place("", 0), "autofix", v) :- option("entryPoint", "print_autofix_hint"), grm_metadata("autofix", v).
.decl grm_message(text: symbol)
grm_message(cat("{\"allowlist\": [", entries, "], \"linter\": \"grammar_render_methods\", \"version\": 1}")) :- option("entryPoint", _),
 entries = join quoted by name sep ", " : { option_item("staticAllowlist", _, name), quoted = @json_str(name) }.
.violation GRMValue(at: span, value: symbol) code "GRM_VALUE" "{value}"
GRMValue(@place("", 0), msg) :- option("entryPoint", "_compute_signature_message"), grm_message(msg).
GRMValue(@place("", 0), key) :- option("entryPoint", "_get_public_key_bytes"), option("publicKey", key).
.violation GRMTrue(at: span) code "GRM_TRUE" "true"
GRMTrue(@place("", 0)) :- option("entryPoint", "_verify_allowlist_integrity"), !option_item("staticAllowlist", _, _).
GRMTrue(@place("", 0)) :- option("entryPoint", "_verify_allowlist_integrity"), option_item("staticAllowlist", _, _),
 option("publicKey", key), option("signature", sig), sig != "NEEDS_SIGNING", grm_message(msg), @ed25519_ok(@replace_all(key, "[\\t-\\r ]", ""), msg, sig) = 1.
GRMTrue(@place("", 0)) :- option("entryPoint", "static_membership"), option("className", cls), option_item("staticAllowlist", _, cls).
'''


def helpers():
    return r'''
.decl grm_kind(n: node, k: symbol)
grm_kind(n, k) :- option_item("astKinds", n, k).
.decl grm_edge(p: node, f: symbol, n: node)
grm_edge(to_number(p), f, n) :- option_item("astParents", n, p), p != "-1", option_item("astFields", n, f).
.decl grm_base(n: node, name: symbol)
grm_base(n, name) :- grm_kind(n, "Name"), option_item("astIds", n, name).
grm_base(n, name) :- grm_kind(n, "Attribute"), option_item("astAttrs", n, name).
grm_base(n, name) :- grm_kind(n, "Subscript"), grm_edge(n, "value", v), grm_base(v, name).
.decl grm_dotted(n: node, name: symbol)
grm_dotted(n, name) :- grm_kind(n, "Name"), option_item("astIds", n, name).
grm_dotted(n, cat(prefix, ".", leaf)) :- grm_kind(n, "Attribute"), grm_edge(n, "value", v), grm_dotted(v, prefix), option_item("astAttrs", n, leaf), v > n.
.decl grm_literal(n: node)
grm_literal(n) :- grm_kind(n, "Constant"), option_item("astValueTypes", n, "str").
.decl grm_string_expr(n: node)
grm_string_expr(n) :- grm_literal(n).
grm_string_expr(n) :- grm_kind(n, "JoinedStr").
GRMValue(@place("", 0), name) :- option("entryPoint", "RenderMethodVisitor._get_name"), grm_base(0, name).
GRMValue(@place("", 0), name) :- option("entryPoint", "RenderMethodVisitor._get_call_name_recursive"), grm_dotted(0, name).
GRMValue(@place("", 0), "") :- option("entryPoint", "RenderMethodVisitor._get_call_name_recursive"), !grm_dotted(0, _).
GRMTrue(@place("", 0)) :- option("entryPoint", e), (e = "RenderMethodVisitor._is_string_expr" ; e = "RenderBodyAnalyzer._is_string_expr"), grm_string_expr(0).
GRMTrue(@place("", 0)) :- option("entryPoint", "RenderBodyAnalyzer._is_string_literal"), grm_literal(0).
.decl grm_line_index(i: number)
grm_line_index(to_number(line)-1) :- option("lineno", line), to_number(line) > 0.
grm_line_index(count+to_number(line)-1) :- option("lineno", line), to_number(line) <= 0, count = count : { option_item("sourceLines", _, _) }.
GRMValue(@place("", 0), raw) :- option("entryPoint", "RenderBodyAnalyzer._get_line_source"), grm_line_index(i), option_item("sourceLines", i, raw).
GRMValue(@place("", 0), "") :- option("entryPoint", "RenderBodyAnalyzer._get_line_source"), !grm_line_index(_).
GRMValue(@place("", 0), "") :- option("entryPoint", "RenderBodyAnalyzer._get_line_source"), grm_line_index(i), !option_item("sourceLines", i, _).
'''


def production():
    # The initial red test has no detector rows. Implement the source event semantics next.
    return r'''
.decl grm_result(path: symbol, line: number, cls: symbol, code: symbol, detail: symbol, ordinal: number)
.violation GRMFact(at: span, class_name: symbol, method_name: symbol, violation_type: symbol, detail: symbol, ordinal: number) code "GRM_FACT" "{detail}"
GRMFact(@place(path, line), cls, "render", code, detail, ordinal) :- option("entryPoint", _), grm_result(path, line, cls, code, detail, ordinal).
.violation GrammarRenderMethod(at: span, cls: symbol, code: symbol, detail: symbol, ordinal: number) code "{code}" "{cls}.render(): {detail}"
GrammarRenderMethod(@place(path, line), cls, code, detail, ordinal) :- !option("entryPoint", _), grm_result(path, line, cls, code, detail, ordinal).
'''
