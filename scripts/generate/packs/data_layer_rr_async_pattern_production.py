"""Expose async-pattern production facts and findings from the existing collector relations."""
import json
import re
from data_layer_rr_text import module


def production(source,text):
    original=module(source,'async_patterns');plugin=original.async_patterns_plugin(source);spec=plugin.required_facts[0];q=json.dumps
    text=re.sub(r'\bdoc\((\w+),\s*(\w+|_),\s*_\)',r'pp_doc(\1,\2)',text)
    text=re.sub(r'(?m)^ap_document\(d, p\) :-[^\n]*!option\("entryPoint", _\)[^\n]*\n','',text)
    text=text.replace('!option("entryPoint", _)', 'pp_run()')
    text=re.sub(r'(?m)^(AsyncASYNC\d+\([^\n]*)(\.\n)',r'\1, !option("entryPoint","_production_contract")\2',text)
    text+='\n#include "data-layer/plugin-production.rr"\n'
    config=dict(name='async_patterns',description=plugin.description,enforcement=plugin.enforcement,findingSeverity='hard-fail',extractor=spec.extractor_id,parser=spec.parser_mode,scope='static',include_tests=str(plugin.file_scope.include_tests).lower(),fact_kind='list')
    text+=''.join(f'pp_config({q(k)},{q(v)}).\n' for k,v in config.items())
    text+=''.join(f'pp_static_root({q(p)}).\n' for p in original.SCAN_PATHS)
    text+=''.join(f'pp_trigger({q(p)}).\n' for p in spec.source_triggers)
    text+='''
ap_document(d,p) :- pp_run(), pp_doc(d,p), pp_fact_present(d).
pp_report(p,l,code,message,p,l,code,0,ordinal,rank,0) :- pp_run(), ap_issue(c,p,code,message,0,rank), ap_call_doc(c,_,p), ap_call_line(c,l), ap_order(c,ordinal).
.violation AsyncPatternProductionFact(at:span,path:symbol,path_order:symbol,order0:number,order1:number,order2:number,f_path:symbol,f_line:number,f_code:symbol,f_message:symbol,f_context:symbol,f_is_fallback:number) code "DL_FACT" "async pattern production fact"
AsyncPatternProductionFact(@place(p,l),p,@replace_all(p,"/","\\u0000"),ordinal,rank,0,p,l,code,message,context,fallback) :- option("entryPoint","_production_contract"),ap_issue(c,p,code,message,fallback,rank),ap_call_doc(c,_,p),ap_call_line(c,l),ap_context(c,context),ap_order(c,ordinal).
'''
    return text
