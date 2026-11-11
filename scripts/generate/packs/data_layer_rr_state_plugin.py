"""Compile State Tripwires plugin metadata and native supplied-fact reporting."""
from dataclasses import asdict
import json
from pathlib import Path
import tempfile


def program(source):
    from data_layer_rr_text import module
    original=module(source,'serverplan_state_tripwires')
    with tempfile.TemporaryDirectory(prefix='state-plugin-author-') as directory:
        plugin=original.serverplan_state_tripwires_plugin(Path(directory))
    metadata=dict(plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description),
        scope=asdict(plugin.file_scope),extractors=[asdict(spec) for spec in plugin.required_facts])
    metadata['scope'].pop('directories')
    text='\n#include "data-layer/signing.rr"\n.decl spp_metadata(json: symbol)\n'
    text+='spp_metadata('+json.dumps(json.dumps(metadata))+').\n'
    for relation,name in [('spp_violation_family','SERVERPLAN_STATE_TRIPWIRE_EXTRACTOR'),('spp_return_family','SERVERPLAN_RETURN_KIND_EXTRACTOR')]:
        text+=f'.decl {relation}(name: symbol)\n{relation}({json.dumps(getattr(original,name).extractor_id)}).\n'
    text+='.decl spp_generated_path(path: symbol)\n'
    text+='spp_generated_path('+json.dumps(original.GENERATED_RETURN_KINDS_PATH.as_posix())+').\n'
    text+='.decl spp_generated_order(ordinal: number,name: symbol)\n'
    for i,name in enumerate(original._load_generated_return_kinds()):
        text+=f'spp_generated_order({i},{json.dumps(name)}).\n'
    from data_layer_rr_text import classes
    text+='.decl spp_whitespace(pattern: symbol)\nspp_whitespace('+json.dumps(classes()[1]+'*')+').\n'
    for relation,field in [('spp_kind','kind'),('spp_text','text'),('spp_parent','parent'),('spp_key_kind','key_kind'),('spp_key','key'),('spp_ordinal','ordinal')]:
        text+=f'.decl {relation}(n: number,value: symbol)\n.input {relation}(IO=json,filename="state-plugin-facts.json",path="/values/*/{field}",projection="python-str")\n'
    for relation,path in [('spp_family_name','/bundles/*/families/*/name'),('spp_family_root','/bundles/*/families/*/root')]:
        text+=f'.decl {relation}(bundle: number,family: number,value: symbol)\n.input {relation}(IO=json,filename="state-plugin-facts.json",path={json.dumps(path)},projection="python-str")\n'
    return text+r'''
.decl spp_plugin_entry()
spp_plugin_entry() :- option("entryPoint","_state_plugin_metadata").
spp_plugin_entry() :- option("entryPoint","_state_plugin_report").
.violation StatePluginMetadata(at: span,json: symbol) code "SPT_PLUGIN_META" "plugin metadata"
StatePluginMetadata(@place("",0),j) :- option("entryPoint","_state_plugin_metadata"),spp_metadata(j).
.violation StatePluginReturnKind(at: span,name: symbol,kind: symbol) code "SPT_PLUGIN_RETURN_KIND" "{name}: {kind}"
StatePluginReturnKind(@place("",0),n,k) :- option("entryPoint","_state_plugin_metadata"),spt_generated(n,k).
.violation StatePluginReturnName(at: span,ordinal: number,name: symbol) code "SPT_PLUGIN_RETURN_NAME" "{name}"
StatePluginReturnName(@place("",0),i,n) :- option("entryPoint","_state_plugin_metadata"),spp_generated_order(i,n).
.decl spp_top(name: symbol)
.decl spp_directory(path: symbol)
spp_directory(".") :- env("state.plugin.metadata","true").
spp_top(n) :- spp_directory(p),dir_entry(p,n,1).
.decl spp_layer(name: symbol)
spp_layer(n) :- spp_top(n),@rx(n,"^l\\d{4}_.+")=1.
.violation StatePluginDirectory(at: span,ordinal: number,path: symbol) code "SPT_PLUGIN_DIR" "plugin directory"
StatePluginDirectory(@place("",0),0,"impl") :- spp_top("impl").
StatePluginDirectory(@place("",0),1,"data_layer") :- spp_top("data_layer").
StatePluginDirectory(@place("",0),2,"_internal") :- spp_top("_internal").
StatePluginDirectory(@place("",0),i+3,n) :- spp_layer(n),i=count : { spp_layer(p),@str_cmp(p,n)<0 }.

.decl spp_edge(p: number,c: number,key_kind: symbol,key: symbol,ordinal: number)
spp_edge(to_number(p),c,k,s,to_number(i)) :- spp_parent(c,p),p!="-1",spp_key_kind(c,k),spp_key(c,s),spp_ordinal(c,i).
.decl spp_bad_json(n: number)
spp_bad_json(n) :- spp_kind(n,k),k!="str",k!="int",k!="float",k!="bool",k!="NoneType",k!="list",k!="tuple",k!="dict".
spp_bad_json(n) :- spp_edge(n,c,_,_,_),spp_bad_json(c).
spp_bad_json(n) :- spp_kind(n,"dict"),spp_edge(n,_,k,_,_),k!="str".
.decl spp_family(b: number,name: symbol,root: number)
spp_family(b,s,to_number(r)) :- spp_family_name(b,i,s),spp_family_root(b,i,r).
.decl spp_return_name(name: symbol)
spp_return_name(s) :- spp_return_family(f),spp_family(_,f,root),spp_kind(root,"dict"),spp_edge(root,n,"str",s,_),spp_kind(n,"list").
.decl spp_return_kind(name: symbol,kind: symbol)
spp_return_kind(s,k) :- spp_return_family(f),spp_family(_,f,root),spp_kind(root,"dict"),spp_edge(root,n,"str",s,_),spp_kind(n,"list"),
    spp_edge(n,v,_,_,_),spp_kind(v,"str"),spp_text(v,k).
.decl spp_stale_generated()
spp_stale_generated() :- spt_generated(n,_),!spp_return_name(n).
spp_stale_generated() :- spp_return_name(n),!spt_generated(n,_).
spp_stale_generated() :- spt_generated(n,k),!spp_return_kind(n,k).
spp_stale_generated() :- spp_return_kind(n,k),!spt_generated(n,k).

// Signature failures precede fact validation and ratchet reads.
allow_entry("state-production",s) :- spp_plugin_entry(),option_item("productionIrEntries",_,s).
allow_sig("state-production","production_ir_allowlist",s) :- spp_plugin_entry(),option("productionIrSignature",s).
multi_name("state-work","native_python_work") :- spp_plugin_entry().
multi_name("state-work","preprocess_runtime_value") :- spp_plugin_entry().
multi_name("state-work","runtime_callbacks") :- spp_plugin_entry().
multi_entry("state-work","native_python_work",s) :- spp_plugin_entry(),option_item("nativeWorkEntries",_,s).
multi_entry("state-work","preprocess_runtime_value",s) :- spp_plugin_entry(),option_item("preprocessEntries",_,s).
multi_entry("state-work","runtime_callbacks",s) :- spp_plugin_entry(),option_item("callbackEntries",_,s).
multi_sig("state-work","lint_serverplan_state_tripwires_python_work",s) :- spp_plugin_entry(),option("pythonWorkSignature",s).
.decl spp_work_allow(key: symbol)
spp_work_allow(s) :- option_item("nativeWorkEntries",_,s).
spp_work_allow(s) :- option_item("preprocessEntries",_,s).
spp_work_allow(s) :- option_item("callbackEntries",_,s).
.decl spp_verified()
spp_verified() :- option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","valid"),multi_status("state-work","valid").
.decl spp_output(ordinal: number,text: symbol)
spp_output(0,cat("FAIL: WARNING: Allowlist for production_ir_allowlist is ",label,". Sign with --sign or a maintainer signing flow.\n")) :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","unsigned"),option("productionIrSignature",s),
    (s="NEEDS_SIGNING",label="needs signing";s!="NEEDS_SIGNING",label="unsigned").
spp_output(0,cat("FAIL: Allowlist signature verification failed for production_ir_allowlist: ",detail,"\n")) :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","invalid"),
    allow_message("state-production",m),option("productionIrSignature",s),nera_key(k),detail=@ed25519_error(k,m,s).
spp_output(1,cat("FAIL: WARNING: Allowlists for lint_serverplan_state_tripwires_python_work are ",label,". Sign with --sign-allowlist.\n")) :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","valid"),multi_status("state-work","unsigned"),
    option("pythonWorkSignature",s),(s="NEEDS_SIGNING",label="needs signing";s!="NEEDS_SIGNING",label="unsigned").
spp_output(1,cat("FAIL: Allowlist signature verification failed for lint_serverplan_state_tripwires_python_work: ",detail,"\n")) :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","valid"),multi_status("state-work","invalid"),
    multi_message("state-work",m),option("pythonWorkSignature",s),nera_key(k),detail=@ed25519_error(k,m,s).
.violation StatePluginOutput(at: span,ordinal: number,text: symbol) code "SPT_PLUGIN_STDERR" "plugin output"
StatePluginOutput(@place("",0),i,t) :- spp_output(i,t).
.decl spp_bad_bundle(b: number)
spp_bad_bundle(b) :- spp_violation_family(f),spp_family(b,f,r),spp_kind(r,k),k!="list".
spp_bad_bundle(b) :- spp_violation_family(f),spp_family(b,f,r),spp_bad_json(r).
.violation StatePluginFactError(at: span,bundle: number,detail: symbol) code "SPT_PLUGIN_FACT_ERROR" "{detail}"
StatePluginFactError(@place("",0),b,"Fact array requires a list of JSON values") :- spp_verified(),spp_bad_bundle(b),b=min k : { spp_bad_bundle(k) }.
.decl spp_field(r: number,name: symbol,kind: symbol,value: symbol)
spp_field(r,name,k,v) :- spp_kind(r,"dict"),spp_edge(r,n,"str",name,_),spp_kind(n,k),spp_text(n,v).
.decl spp_line(r: number,json: symbol)
spp_line(r,v) :- spp_field(r,"line","int",v).
spp_line(r,v) :- spp_field(r,"line","bool",v).
.decl spp_path_root(raw: symbol,prefix: symbol)
spp_path_root(p,"") :- spp_field(_,"file","str",p),!prefix("/",p).
spp_path_root(p,"/") :- spp_field(_,"file","str",p),prefix("/",p),!prefix("//",p).
spp_path_root(p,"/") :- spp_field(_,"file","str",p),prefix("///",p).
spp_path_root(p,"//") :- spp_field(_,"file","str",p),prefix("//",p),!prefix("///",p).
.decl spp_path_body(raw: symbol,body: symbol)
spp_path_body(p,body) :- spp_path_root(p,_),body=join part by i sep "/" : { match_all("(?<part>[^/]+)",p,i,_,_),part!="." }.
.decl spp_path(raw: symbol,path: symbol)
spp_path(p,cat(root,body)) :- spp_path_root(p,root),spp_path_body(p,body),(root!="";body!="").
spp_path(p,".") :- spp_path_root(p,""),spp_path_body(p,"").
.decl spp_raw(b: number,i: number,code: symbol,path: symbol,line_json: symbol,qualname: symbol,message: symbol)
spp_raw(b,i,c,p,l,q,m) :- spp_verified(),!spp_bad_bundle(_),spp_violation_family(f),spp_family(b,f,root),spp_edge(root,r,_,_,i),
    spp_field(r,"code","str",c),spp_field(r,"file","str",raw),spp_path(raw,p),spp_line(r,l),spp_field(r,"qualname","str",q),spp_field(r,"message","str",m).
.decl spp_relative(path: symbol,relative: symbol)
spp_relative(p,p) :- spp_raw(_,_,_,p,_,_,_),!prefix("/",p).
spp_relative(p,r) :- spp_raw(_,_,_,p,_,_,_),option("moduleRoot",root),prefix(cat(root,"/"),p),r=@sql_substr2(p,@sql_length(root)+2).
spp_relative(p,r) :- spp_raw(_,_,_,p,_,_,_),option("moduleRoot",root),prefix("/",p),!prefix(cat(root,"/"),p),
    r=@rx_extract(p,"(?:^|/)((?:impl|data_layer|_internal|l[0-9]{4}_[^/]+)(?:/.*|$))",1).
spp_relative(p,p) :- spp_raw(_,_,_,p,_,_,_),option("moduleRoot",root),prefix("/",p),!prefix(cat(root,"/"),p),
    @rx(p,"(?:^|/)(?:impl|data_layer|_internal|l[0-9]{4}_[^/]+)(?:/|$)")=0.
.decl spp_retained(b: number,i: number,key: symbol)
spp_retained(b,i,key) :- spp_raw(b,i,c,p,_,q,_),spp_relative(p,r),key=cat(c,"::",q,"::",r),!spp_work_allow(key).
.decl spp_ratchet_path(path: symbol)
// World requests consume exported settings before runtime option relations.
// Keep the physical read key independent of the post-load reporting stage.
spp_ratchet_path(p) :- env("state.plugin.allowlist",p).
.decl spp_ratchet_entry(line: number,fragment: number,key: symbol)
spp_ratchet_entry(i,j,key) :- spp_verified(),!spp_bad_bundle(_),spp_ratchet_path(p),file_line(p,i,raw),
    match_all("(?<part>[^\\n\\r\\v\\f\\x{1c}-\\x{1e}\\x{85}\\x{2028}\\x{2029}]+)",raw,j,_,_),
    spp_whitespace(w),key=@replace_all(part,cat("^",w,"|",w,"$"),""),key!="",!prefix("#",key).
.decl spp_allow_count(key: symbol,n: number)
spp_allow_count(k,n) :- spp_retained(_,_,k),n=count : { spp_ratchet_entry(_,_,k) }.
.decl spp_current_count(key: symbol,n: number)
spp_current_count(k,n) :- spp_ratchet_entry(_,_,k),n=count : { spp_retained(_,_,k) }.
.decl spp_first(key: symbol,bundle: number,ordinal: number)
spp_first(k,b,i) :- spp_retained(b,i,k),!spp_retained_before(k,b,i,_,_).
.decl spp_retained_before(key: symbol,bundle: number,ordinal: number,earlier_bundle: number,earlier_ordinal: number)
spp_retained_before(k,b,i,a,j) :- spp_retained(b,i,k),spp_retained(a,j,k),(a<b;a=b,j<i).
.decl spp_key_rank(key: symbol,rank: number)
spp_key_rank(k,r) :- spp_first(k,b,i),r=count : { spp_first(_,a,j),(a<b;a=b,j<i) }.
.decl spp_report(phase: number,group_order: number,bundle: number,ordinal: number,code: symbol,path: symbol,line_json: symbol,message: symbol)
spp_report(0,0,0,0,"SPT-GENERATED-RETURN-KINDS",p,"1","generated ServerPlan return kinds are stale; run the return-kind generator and rerun lint") :-
    option("entryPoint","_state_plugin_report"),spp_stale_generated(),option("generatedPath",p).
spp_report(0,0,0,0,"SPT-ALLOWLIST",p,"1","production IR allowlist integrity verification failed") :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production",s),s!="valid",option("linterPath",p).
spp_report(0,0,0,0,"SPT-ALLOWLIST",p,"1","ServerPlan Python-work allowlist signature verification failed") :-
    option("entryPoint","_state_plugin_report"),!spp_stale_generated(),allow_status("state-production","valid"),multi_status("state-work",s),s!="valid",option("linterPath",p).
spp_report(1,g,b,i,c,p,l,cat(q,": ",m)) :- spp_retained(b,i,k),spp_allow_count(k,allowed),spp_key_rank(k,g),
    before=count : { spp_retained_before(k,b,i,_,_) },before>=allowed,spp_raw(b,i,c,p,l,q,m).
.decl spp_stale_key(key: symbol)
spp_stale_key(key) :-
    spp_ratchet_entry(_,_,key),spp_current_count(key,current),allowed=count : { spp_ratchet_entry(_,_,key) },current<allowed,
    spp_verified().
spp_report(2,g,0,0,"SPT-STALE-ALLOWLIST",p,"1",cat("stale allowlist entry should be removed: ",key)) :-
    spp_stale_key(key),g=count : { spp_stale_key(other),@str_cmp(other,key)<0 },option("allowlistPath",p).
.violation StatePluginReport(at: span,phase: number,group_order: number,bundle: number,ordinal: number,source_code: symbol,path: symbol,line_json: symbol,message: symbol) code "SPT_PLUGIN_REPORT" "{message}"
StatePluginReport(@place("",0),a,g,b,i,c,p,l,m) :- spp_report(a,g,b,i,c,p,l,m).
.violation StatePluginReadError(at: span,path: symbol,io_kind: symbol,errno: number,detail: symbol,bytes_hex: symbol,start: number,end: number,reason: symbol) code "SPT_PLUGIN_IO_ERROR" "allowlist read failed"
StatePluginReadError(@place("",0),p,k,n,d,b,s,e,r) :- spp_verified(),!spp_bad_bundle(_),spp_ratchet_path(p),file_error(p,k,n,d,b,s,e,r).
'''
