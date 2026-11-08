"""Translate async/sync delegation using shared native syntax and typed AST arguments."""
import re


def async_sync_delegation(source,target,header):
    text=header('async_sync_delegation',target)+'#include "python/print.rr"\n'
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    return text+configuration()+helpers()+analysis()


def configuration():
    return r'''
.violation ASDConfig(at: span, name: symbol, value: symbol) code "ASD_CONFIG" "{value}"
ASDConfig(@place("", 0), "LINTER_NAME", "async_sync_delegation_category_c") :- option("entryPoint", "configuration").
ASDConfig(@place("", 0), "ALLOWLIST_SIGNATURE", "") :- option("entryPoint", "configuration").
'''


def helpers():
    return r'''
.decl asdh_kind(n: node, kind: symbol)
asdh_kind(n, k) :- option_item("astKinds", n, k).
.decl asdh_edge(n: node, field: symbol, c: node)
asdh_edge(to_number(parent), field, c) :- option_item("astParents", c, parent), parent != "-1", option_item("astFields", c, field).
.violation ASDTrue(at: span) code "ASD_TRUE" "true"
ASDTrue(@place("", 0)) :- option("entryPoint", _), 1 = 0.
.violation ASDValue(at: span, value: symbol) code "ASD_VALUE" "{value}"
ASDValue(@place("", 0), "") :- option("entryPoint", _), 1 = 0.
.violation ASDNode(at: span, n: node, ordinal: number) code "ASD_NODE" "node"
ASDNode(@place("", 0), n, 0) :- asdh_kind(n, _), 1 = 0.
.violation ASDRequirement(at: span, base: symbol, lazy: symbol, counterpart: symbol, decorator: symbol) code "ASD_REQUIREMENT" "requirements"
ASDRequirement(@place("", 0), "", "", "", "") :- option("entryPoint", _), 1 = 0.
'''


def analysis():
    return r'''
.violation ASDParse(at: span, error: symbol) code "ASD_PARSE" "{error}"
ASDParse(@place(path, 1), cat("SyntaxError: ", message, " (", name, ", line ", to_string(l), ")")) :- option("entryPoint", "analyze_source"), doc(d, _, _), syntax_error(d, l, message), option("originalPath", path), option("originalName", name).
.violation ASDAnalysis(at: span, function_name: symbol, category: symbol, description: symbol, sync: symbol, details: symbol, has_sync: number, has_details: number, phase: number, ordinal: number) code "ASD_ANALYSIS" "{description}"
ASDAnalysis(@place(path, 1), "", "", "", "", "", 0, 0, 0, 0) :- option("originalPath", path), 1 = 0.
'''
