"""Expose complete production boundary facts from the same owner-aware detector relations."""
import json
from data_layer_rr_text import module


def production(source):
    original=module(source,'async_sync_boundary');plugin=original.async_sync_boundary_plugin(source);spec=plugin.required_facts[0];q=json.dumps
    config=dict(name=plugin.name,description=plugin.description,enforcement=plugin.enforcement,findingSeverity='hard-fail',extractor=spec.extractor_id,parser=spec.parser_mode,scope='static',include_tests=str(plugin.file_scope.include_tests).lower(),fact_kind='list')
    text=''.join(f'pp_config({q(k)},{q(v)}).\n' for k,v in config.items())
    text+=''.join(f'pp_static_root({q(p.relative_to(source).as_posix())}).\n' for p in plugin.file_scope.directories)
    text+=''.join(f'pp_trigger({q(p)}).\n' for p in spec.source_triggers)
    text+='''
pp_report(p,l,code,cat(label,": ",fn,": ",text),p,l,code,0,i,0,0) :- pp_run(),bs_emit(c,p,code,fn,_,text),line(c,l),visit_index(c,i),bs_label(code,label).
.violation BoundaryProductionFact(at:span,path:symbol,path_order:symbol,order0:number,order1:number,order2:number,f_file:symbol,f_line:number,f_col:number,f_category:symbol,f_function_name:symbol,f_call_text:symbol,f_is_async_ctx:number) code "DL_FACT" "boundary production fact"
BoundaryProductionFact(@place(p,l),p,@replace_all(p,"/","\\u0000"),i,0,0,p,l,col,code,fn,text,a) :- option("entryPoint","_production_contract"),bs_detect(c,code,fn,a,text),docof(c,d),bs_document(d,p),line(c,l),col(c,col),visit_index(c,i).
'''
    return text
