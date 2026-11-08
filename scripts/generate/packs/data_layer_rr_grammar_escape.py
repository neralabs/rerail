"""Generate grammar escape-hatch relations from the pinned source constants."""
import json
from data_layer_rr_text import module

GROUPS = ['ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS','_SAFE_PARAM_TYPES_STATIC',
          'SAFE_PARAM_TYPES','TRANSPARENT_CONTAINER_TYPES','SCHEMA_METADATA_CLASSES',
          'CALLSITE_PROTECTED_CLASSES','FORBIDDEN_ESCAPE_HATCHES']


def sql_grammar_escape_hatches(source,target,header):
    original=module(source,'sql_grammar_escape_hatches')
    text=header('sql_grammar_escape_hatches',target)+'\n.decl sgeh_constant(family: symbol, value: symbol)\n'
    for group in GROUPS:
        for value in sorted(getattr(original,group)):
            text+='sgeh_constant('+json.dumps(group)+','+json.dumps(value)+').\n'
    text+='.decl sgeh_metadata(key: symbol, value: symbol)\n'
    for key in ['ALLOWLIST_PUBLIC_KEY_HEX','ALLOWLIST_SIGNATURE','MAX_ESCAPE_HATCH_VIOLATIONS']:
        text+='sgeh_metadata('+json.dumps(key)+','+json.dumps(str(getattr(original,key)))+').\n'
    text+='''
.violation SGEConstant(at: span, family: symbol, value: symbol) code "SGE_CONSTANT" "{value}"
SGEConstant(@place("", 0), f, v) :- option("entryPoint", "configuration"), sgeh_constant(f, v).
.violation SGEMetadata(at: span, key: symbol, value: symbol) code "SGE_METADATA" "{value}"
SGEMetadata(@place("", 0), k, v) :- option("entryPoint", "configuration"), sgeh_metadata(k, v).
'''
    return text
