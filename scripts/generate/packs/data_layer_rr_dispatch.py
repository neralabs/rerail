"""Compile dispatch constants and finite casefold vocabulary from pinned Python, offline."""
import json
import re
from functools import lru_cache
from data_layer_rr_text import module,classes
from data_layer_rr_cli import api_scope


def folded_pattern(words):
    # casefold is a per-codepoint mapping. Include multichar folds such as the st ligature.
    images={}
    for point in range(0x110000):
        char=chr(point);fold=char.casefold()
        if fold and any(fold in word for word in words):images.setdefault(fold,[]).append(char)
    def word_pattern(word):
        @lru_cache(None)
        def suffix(i):
            if i==len(word):return ''
            parts=['['+''.join(re.escape(c) for c in chars)+']'+suffix(i+len(fold))
                for fold,chars in images.items() if word.startswith(fold,i)]
            return '(?:'+'|'.join(parts)+')'
        return suffix(0)
    return '(?:'+'|'.join(word_pattern(word) for word in sorted(words))+')'


def generate(source,target,header):
    original=module(source,'serverplan_dispatch_architecture');text=api_scope(header('serverplan_dispatch_architecture',target))
    for relation,name in [('sd_family','_BACKEND_FAMILY_ATTRS'),('sd_backend_call','_BACKEND_CALLS'),
        ('sd_step_name','_STEP_TYPE_NAMES'),('sd_nonstep','_NON_DISPATCH_STEP_TYPES'),('sd_step_suffix','_STEP_TYPE_SUFFIXES'),
        ('sd_root','_GENERIC_DIRS'),('sd_late','_LATE_BOUNDARY_FILES')]:
        text+=f'.decl {relation}(value: symbol)\n'
        text+=''.join(f'{relation}({json.dumps(v)}).\n' for v in sorted(getattr(original,name)))
    text+='.decl sd_backend_pattern(value: symbol)\nsd_backend_pattern('+json.dumps(folded_pattern(original._BACKEND_LITERAL_VALUES))+').\n'
    messages={'SPD001':'backend capability selection belongs at the late physical boundary',
        'SPD002':'semantic dispatch must use typed plan contracts, not concrete step types',
        'SPD003':'semantic dispatch must not compare runtime step type names','SPD004':original._DIALECT_PRESENCE_GUIDANCE}
    text+='.decl sd_message(code: symbol, value: symbol)\n'
    text+=''.join(f'sd_message({json.dumps(k)},{json.dumps(v)}).\n' for k,v in messages.items())
    text+='.decl sd_base(value: symbol)\nsd_base('+json.dumps(original._BASE_SERVER_PLAN_FILE)+').\n'
    text+='.decl sd_whitespace(value: symbol)\nsd_whitespace('+json.dumps(classes()[1]+'+')+').\n'
    return text+'#include "data-layer/dispatch-source.rr"\n'
