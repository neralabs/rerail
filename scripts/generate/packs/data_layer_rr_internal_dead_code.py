"""Translate dead-code survey decisions over the shared native graph contract."""
import re


def internal_dead_code(source,target,header):
    text=header('internal_dead_code',target)
    text=re.sub(r'^\.scope include .*$', '.scope include "**/*.py"',text,flags=re.M)
    text=re.sub(r'^\.scope exclude .*$', '.scope exclude',text,flags=re.M)
    return text+'#include "data-layer/survey-graph.rr"\n'+empty_outputs()


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
