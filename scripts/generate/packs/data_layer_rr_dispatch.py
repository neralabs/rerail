"""Compile dispatch constants and finite casefold vocabulary from pinned Python, offline."""
import json
import re
from dataclasses import asdict
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
    text+='.scope text "scripts/allowlists/lint_serverplan_dispatch_architecture.txt"\n'
    for relation,name in [('sd_family','_BACKEND_FAMILY_ATTRS'),('sd_backend_call','_BACKEND_CALLS'),
        ('sd_step_name','_STEP_TYPE_NAMES'),('sd_nonstep','_NON_DISPATCH_STEP_TYPES'),('sd_step_suffix','_STEP_TYPE_SUFFIXES'),
        ('sd_root','_GENERIC_DIRS'),('sd_late','_LATE_BOUNDARY_FILES'),
        ('sdp_api','_BASE_SERVER_PLAN_EXECUTOR_API'),('sdp_consumer','_BASE_SERVER_PLAN_EXECUTOR_CONSUMERS'),
        ('sdp_reflective','_REFLECTIVE_ACCESS_CALLS'),('sdp_forbidden','_EXECUTOR_REACH_THROUGH_ATTRS')]:
        text+=f'.decl {relation}(value: symbol)\n'
        text+=''.join(f'{relation}({json.dumps(v)}).\n' for v in sorted(getattr(original,name)))
    text+='.decl sd_backend_pattern(value: symbol)\nsd_backend_pattern('+json.dumps(folded_pattern(original._BACKEND_LITERAL_VALUES))+').\n'
    messages={'SPD001':'backend capability selection belongs at the late physical boundary',
        'SPD002':'semantic dispatch must use typed plan contracts, not concrete step types',
        'SPD003':'semantic dispatch must not compare runtime step type names','SPD004':original._DIALECT_PRESENCE_GUIDANCE,
        'SPD005':original._EXECUTOR_REACH_THROUGH_GUIDANCE}
    text+='.decl sdp_factory_pattern(value: symbol)\nsdp_factory_pattern('+json.dumps('.*'+folded_pattern({'backend_executor'})+'.*')+').\n'
    text+='.decl sd_message(code: symbol, value: symbol)\n'
    text+=''.join(f'sd_message({json.dumps(k)},{json.dumps(v)}).\n' for k,v in messages.items())
    text+='.decl sd_base(value: symbol)\nsd_base('+json.dumps(original._BASE_SERVER_PLAN_FILE)+').\n'
    text+='.decl sd_whitespace(value: symbol)\nsd_whitespace('+json.dumps(classes()[1]+'+')+').\n'
    text+='#include "data-layer/dispatch-source.rr"\n#include "data-layer/dispatch-repository.rr"\n'
    plugin=original.dispatch_architecture_plugin(source)
    config=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
        findingSeverity='hard-fail',extractor=original.DISPATCH_EXTRACTOR.extractor_id,parser='ast_collect',scope='static',include_tests='false',fact_kind='list')
    text+=''.join(f'pp_config({json.dumps(k)},{json.dumps(v)}).\n' for k,v in config.items())
    text+=''.join(f'pp_static_root({json.dumps(v)}).\n' for v in original._GENERIC_DIRS)
    text+='.decl sdr_root_scope(ordinal: number,path: symbol,parent: symbol,name: symbol)\n'
    for i,value in enumerate(original._GENERIC_DIRS):
        parent,name=value.rsplit('/',1) if '/' in value else ('.',value)
        text+=f'sdr_root_scope({i},{json.dumps(value)},{json.dumps(parent)},{json.dumps(name)}).\n'
    text+='.decl sdr_default_signature(value: symbol)\nsdr_default_signature('+json.dumps(original.INVENTORY_SIGNATURE)+').\n'
    for section,fields in [('plugin',dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)),
        ('scope',dict(include_tests=False,broad_repo_scope=False)),('extractor',asdict(original.DISPATCH_EXTRACTOR))]:
        for ordinal,(key,value) in enumerate(fields.items()):
            if isinstance(value,str):kind,encoded='string',value
            elif isinstance(value,bool):kind,encoded='boolean',str(value).lower()
            elif value is None:kind,encoded='none',''
            elif isinstance(value,tuple):kind,encoded='tuple',''
            else:raise ValueError(f'Unrepresented dispatch metadata {section}.{key}: {type(value)}')
            text+=f'sdr_config({json.dumps(section)},{json.dumps(key)},{ordinal},{json.dumps(kind)},{json.dumps(encoded)}).\n'
            if kind=='tuple':
                if not all(isinstance(v,str) for v in value):raise ValueError('Unrepresented metadata tuple members')
                for i,member in enumerate(value):text+=f'sdr_sequence({json.dumps(section)},{json.dumps(key)},{i},"string",{json.dumps(member)}).\n'
    return text
