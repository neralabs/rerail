"""Bind production discovery and facts to the same dynamic-loading hit relation."""
import json
import re
from data_layer_rr_text import module


def augment(source,text):
    original=module(source,'dynamic_module_loading');plugin=original.dynamic_module_loading_plugin(source);spec=plugin.required_facts[0]
    config=dict(name=plugin.name,description=plugin.description,enforcement=plugin.enforcement,findingSeverity='hard-fail',extractor=spec.extractor_id,parser=spec.parser_mode,scope='production',include_tests='false',fact_kind='list',layer_regex=r'l\d{4}_[^\n]+(?s:.*)')
    text=re.sub(r'^\.description .*$', '.description '+json.dumps(plugin.description),text,flags=re.M)
    text=text.replace('dml_scanning() :- !option("entryPoint",_).\n','')
    old='''DynamicSignature(@place("scripts/lint_dynamic_module_loading.py", 1)) :- !option("entryPoint", _),
    allow_status("dynamic_module_loading", status), status != "valid".\n'''
    assert old in text;text=text.replace(old,'')
    text+='#include "data-layer/dynamic-production.rr"\n'
    for k,v in config.items():text+=f'pp_config({json.dumps(k)},{json.dumps(v)}).\n'
    for trigger in spec.source_triggers:text+=f'pp_trigger({json.dumps(trigger)}).\n'
    return text
