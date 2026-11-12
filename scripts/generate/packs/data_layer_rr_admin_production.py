"""Expose complete admin production facts and source reporter order from shared detections."""
import json
from data_layer_rr_text import module


def production(source):
    original=module(source,'admin_async_purity');plugin=original.admin_async_purity_plugin(source);spec=plugin.required_facts[0];q=json.dumps
    config=dict(name=plugin.name,description=plugin.description,enforcement=plugin.enforcement,extractor=spec.extractor_id,
        parser=spec.parser_mode,scope='static',include_tests=str(plugin.file_scope.include_tests).lower(),fact_kind='admin-violations')
    text=''.join(f'pp_config({q(k)},{q(v)}).\n' for k,v in config.items())
    text+=''.join(f'pp_static_root({q(p.relative_to(source).as_posix())}).\n' for p in plugin.file_scope.directories)
    text+=''.join(f'pp_trigger({q(value)}).\n' for value in spec.source_triggers)
    return text+'''
// Facts preserve line/pattern order. Structured findings use stable_key's string order.
pp_report(p,l,"AAP001",cat(pattern,": ",text),cat(p,":",to_string(l),":",pattern,": ",text),0,"",0,l,ordinal,0) :- pp_run(),admin_result(_,p,l,ordinal,pattern,text).
.violation AdminProductionFact(at:span,path:symbol,path_order:symbol,order0:number,order1:number,order2:number,f_file:symbol,f_line:number,f_pattern:symbol,f_text:symbol) code "DL_FACT" "admin production fact"
AdminProductionFact(@place(p,l),p,@replace_all(p,"/","\\u0000"),l,ordinal,0,p,l,pattern,text) :- option("entryPoint","_production_contract"),admin_result(_,p,l,ordinal,pattern,text).
'''
