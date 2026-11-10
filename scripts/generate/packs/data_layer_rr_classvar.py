"""Generate ClassVar API metadata and typed external-value relations."""
from dataclasses import asdict,fields
import json
from pathlib import Path
import tempfile
from data_layer_rr_text import module


def suffix(source):
    original=module(source,'classvar_check')
    with tempfile.TemporaryDirectory(prefix='classvar-metadata-') as temp:
        plugin=original.classvar_check_plugin(Path(temp))
        metadata={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name not in {'reporter','required_facts','file_scope'}}
        scope=asdict(plugin.file_scope);scope.pop('directories')
    q=json.dumps
    config=dict(docstring=original.__doc__,extractor=json.dumps(asdict(original.CLASSVAR_CHECK_EXTRACTOR)),plugin=json.dumps(metadata),file_scope=json.dumps(scope))
    text='\n.decl cv_config(key: symbol,value: symbol)\n'
    text+=''.join(f'cv_config({q(k)},{q(v)}).\n' for k,v in config.items())
    text+=r'''
.violation ClassVarConfig(at: span,key: symbol,value: symbol) code "CV_CONFIG" "ClassVar configuration"
ClassVarConfig(@place("",0),k,v) :- option("entryPoint","_classvar_configuration"),cv_config(k,v).
.decl cv_plugin_dir(path: symbol, rank: number)
cv_plugin_dir(p,0) :- dir_entry(".",p,1), p="impl".
cv_plugin_dir(p,1) :- dir_entry(".",p,1), p="data_layer".
cv_plugin_dir(p,2) :- dir_entry(".",p,1), p="_internal".
cv_plugin_dir(p,3) :- dir_entry(".",p,1), pp_config("layer_regex",rx),match(rx,p).
.violation ClassVarPluginDir(at: span,path: symbol,rank: number) code "CV_PLUGIN_DIR" "ClassVar plugin directory"
ClassVarPluginDir(@place("",0),p,r) :- option("entryPoint","_classvar_plugin_metadata"),cv_plugin_dir(p,r).

.decl cv_ast(n: number,kind: symbol)
cv_ast(n,k) :- option_item("astKinds",n,k).
.decl cv_ast_child(p: number,n: number,field: symbol,ordinal: number)
cv_ast_child(to_number(raw),n,f,to_number(i)) :- option_item("astParents",n,raw),raw!="-1",option_item("astFields",n,f),option_item("astOrdinals",n,i).
.decl cv_ast_ancestor(n: number,a: number)
cv_ast_ancestor(n,p) :- cv_ast_child(p,n,_,_).
cv_ast_ancestor(n,a) :- cv_ast_child(p,n,_,_),cv_ast_ancestor(p,a).
.decl cv_ast_leaf(n: number,name: symbol)
cv_ast_leaf(n,s) :- cv_ast(n,"Name"),option_item("astIds",n,s).
cv_ast_leaf(n,s) :- cv_ast(n,"Attribute"),option_item("astAttrs",n,s).
.decl cv_ast_helper(n: number)
cv_ast_helper(n) :- cv_ast(n,"FunctionDef"),option_item("astNames",n,"is_classvar_annotation").
.decl cv_ast_exempt(n: number)
cv_ast_exempt(n) :- cv_ast_ancestor(n,f),cv_ast_helper(f).
.decl cv_ast_style(c: number,i: number,style: symbol)
cv_ast_style(c,i,"is/==") :- cv_ast_child(c,n,"ops",i),cv_ast(n,k),(k="Is";k="Eq").
cv_ast_style(c,i,"is not/!=") :- cv_ast_child(c,n,"ops",i),cv_ast(n,k),(k="IsNot";k="NotEq").
.violation ClassVarAstFact(at: span,visit: number,ordinal: number,file: symbol,line: number,column: number,style: symbol) code "CV_AST_FACT"
    "Direct get_origin(...) {style} ClassVar check. This fails for string annotations (PEP 563). Use is_classvar_annotation() from l1000_schema.type_utils instead."
ClassVarAstFact(@place("",0),c,i,p,to_number(l),to_number(col),s) :- option("entryPoint","_classvar_ast_collect"),option("cvFactPath",p),
    cv_ast(c,"Compare"),cv_ast_child(c,left,"left",0),cv_ast(left,"Call"),cv_ast_child(left,fn,"func",0),cv_ast_leaf(fn,"get_origin"),
    cv_ast_style(c,i,s),cv_ast_child(c,other,"comparators",i),cv_ast_leaf(other,"ClassVar"),!cv_ast_exempt(c),
    option_item("astLines",c,l),option_item("astColumns",c,col).

.decl cv_project_file(i: number,value: symbol)
.input cv_project_file(IO=json,filename="classvar-values.json",path="/values/*/file",projection="json")
.decl cv_project_line(i: number,value: symbol)
.input cv_project_line(IO=json,filename="classvar-values.json",path="/values/*/line",projection="json")
.decl cv_project_col(i: number,value: symbol)
.input cv_project_col(IO=json,filename="classvar-values.json",path="/values/*/col",projection="json")
.decl cv_project_message(i: number,value: symbol)
.input cv_project_message(IO=json,filename="classvar-values.json",path="/values/*/message",projection="json")
.decl cv_project_code(i: number,value: symbol)
.input cv_project_code(IO=json,filename="classvar-values.json",path="/values/*/code",projection="json")
.violation ClassVarProjection(at: span,ordinal: number,value_json: symbol) code "CV_PROJECT" "ClassVar fact projection"
ClassVarProjection(@place("",0),i,cat("{\"file\":",f,",\"line\":",l,",\"col\":",c,",\"message\":",m,",\"code\":",k,"}")) :-
    option("entryPoint","_classvar_project"),cv_project_file(i,f),cv_project_line(i,l),cv_project_col(i,c),cv_project_message(i,m),cv_project_code(i,k).
#include "data-layer/typed-supplied-facts.rr"
'''
    ns=original.CLASSVAR_CHECK_EXTRACTOR.extractor_id.replace('~','~0').replace('/','~1');prefix='/bundles/*/facts/'+ns
    for rel,path,projection in [('sfe_container',prefix+'/kind','python-str'),('sfe_shape',prefix+'/rows/*/kind','python-str'),
        ('sfe_member',prefix+'/rows/*/fields/*/key','python-str'),('sfe_value',prefix+'/rows/*/fields/*/encoded','python-value-str'),
        ('sfe_tag',prefix+'/rows/*/fields/*/encoded/0','python-str'),('sfe_scalar',prefix+'/rows/*/fields/*/encoded/1','python-str'),
        ('sfe_json',prefix+'/rows/*/fields/*/encoded/1','json')]:
        text+=f'.input {rel}(IO=json,filename="reporter-facts.json",path={q(path)},projection={q(projection)})\n'
    return text


def batch_suffix():
    return r'''
.decl cv_prepare_path(i: number,p: symbol,scope: symbol)
cv_prepare_path(i,p,s) :- option_item("cvCandidatePaths",i,p),option_item("cvScopePaths",i,s).
.decl cv_prepare_test(i: number)
cv_prepare_test(i) :- cv_prepare_path(i,_,s),(contains("/tests/",cat("/",s));contains("/test/",cat("/",s))).
.decl cv_prepare_previous(i: number)
cv_prepare_previous(i) :- cv_prepare_path(i,p,_),cv_prepare_path(j,p,_),j<i.
.decl cv_prepare_gate(i: number)
cv_prepare_gate(i) :- cv_prepare_path(i,_,_),option("cvNeedsSourceGate","0").
cv_prepare_gate(i) :- cv_prepare_path(i,_,_),option("cvNeedsSourceGate","1"),option_item("cvCallSources",i,s),contains("get_origin",s),contains("ClassVar",s).
.violation ClassVarRead(at: span,ordinal: number,path: symbol) code "CV_READ" "ClassVar file read"
ClassVarRead(@place("",0),i,p) :- option("entryPoint","_classvar_prepare"),cv_prepare_path(i,p,s),suffix(".py",s),s!=".py",!suffix("/.py",s),
    option_item("cvCandidateExists",i,"1"),!cv_prepare_test(i),!cv_prepare_previous(i),cv_prepare_gate(i).
.decl cv_walk_excluded(i: number)
cv_walk_excluded(i) :- option_item("cvWalkPaths",i,p),option_item("cvExclude",_,s),contains(s,p).
cv_walk_excluded(i) :- option_item("cvWalkRelative",i,p),contains("/__pycache__/",cat("/",p)).
.violation ClassVarWalkFile(at: span,ordinal: number,path: symbol) code "CV_WALK_FILE" "ClassVar directory candidate"
ClassVarWalkFile(@place("",0),i,p) :- option("entryPoint","_classvar_walk_files"),option_item("cvWalkPaths",i,p),suffix(".py",p),!cv_walk_excluded(i).
'''


def cli_suffix(source):
    import ast
    from data_layer_rr_dynamic_cli import literal
    tree=ast.parse((source/'scripts/lint_classvar_check.py').read_text());fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='cli_main')
    parser=next(n for n in ast.walk(fn) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='ArgumentParser')
    description=literal(next(k.value for k in parser.keywords if k.arg=='description'))
    discovery=ast.parse((source/'scripts/lint_lib/discovery.py').read_text());args_fn=next(n for n in discovery.body if isinstance(n,ast.FunctionDef) and n.name=='add_discovery_args')
    calls=[n.value for body in [fn.body,args_fn.body] for n in body if isinstance(n,ast.Expr) and isinstance(n.value,ast.Call) and isinstance(n.value.func,ast.Attribute) and n.value.func.attr=='add_argument']
    schema=[dict(name=literal(c.args[0]),kwargs={k.arg:(k.value.id if isinstance(k.value,ast.Name) else literal(k.value)) for k in c.keywords}) for c in calls]
    text='\ncv_config("cli_schema",'+json.dumps(json.dumps(dict(description=description,arguments=schema)))+').\n'
    text+=r'''
#include "data-layer/python-discovery-roots.rr"
sgd_enabled() :- env("cv.cli.dirs","1").
.violation ClassVarCliRoot(at: span,path: symbol) code "CV_ROOT" "ClassVar CLI root"
ClassVarCliRoot(@place("",0),p) :- option("entryPoint","_classvar_cli_root"),option("cvHasRoot","1"),option("cvRoot",p).
ClassVarCliRoot(@place("",0),@dir(@dir(@dir(p)))) :- option("entryPoint","_classvar_cli_root"),option("cvHasRoot","0"),option("cvDiscoverySource",p).
.violation ClassVarCliDirectoryArgument(at: span,ordinal: number,path: symbol) code "CV_DIR_ARG" "ClassVar directory argument"
ClassVarCliDirectoryArgument(@place("",0),i,p) :- option("entryPoint","_classvar_cli_root"),option("cvHasDirs","1"),option_item("cvDirs",i,p).
ClassVarCliDirectoryArgument(@place("",0),i,p) :- option("entryPoint","_classvar_cli_root"),option("cvHasDirs","0"),option("cvHasDefaults","1"),option_item("cvDefaults",i,p).
ClassVarCliDirectoryArgument(@place("",0),0,"public") :- option("entryPoint","_classvar_cli_root"),option("cvHasDirs","0"),option("cvHasDefaults","0").
.violation ClassVarCliDirectory(at: span,ordinal: number,path: symbol) code "CV_DIRECTORY" "ClassVar CLI directory"
ClassVarCliDirectory(@place("",0),i,p) :- option("entryPoint","_classvar_cli_dirs"),sgd_chosen(a,b,p),i=count:{sgd_chosen(c,d,_),(c<a;c=a,d<b)}.
.violation ClassVarCliPath(at: span,ordinal: number,path: symbol) code "CV_CLI_PATH" "ClassVar CLI path"
ClassVarCliPath(@place("",0),i,p) :- option("entryPoint","_classvar_cli_paths"),option_item("cvRequestedPaths",i,p).
ClassVarCliPath(@place("",0),i,p) :- option("entryPoint","_classvar_cli_paths"),!option_item("cvRequestedPaths",_,_),option_item("cvDefaultPaths",i,p).
.decl cv_cli_excluded(i: number)
cv_cli_excluded(i) :- option_item("cvCliPaths",i,p),(contains("tests/",p);contains("test_",p);contains("__pycache__",p);contains(".pyc",p)).
.violation ClassVarCliFile(at: span,ordinal: number,path: symbol) code "CV_CLI_FILE" "ClassVar CLI file"
ClassVarCliFile(@place("",0),i,p) :- option("entryPoint","_classvar_cli_prepare"),option_item("cvCliPaths",i,p),option_item("cvCliKinds",i,"direct").
ClassVarCliFile(@place("",0),i,p) :- option("entryPoint","_classvar_cli_prepare"),option_item("cvCliPaths",i,p),option_item("cvCliKinds",i,"walk"),suffix(".py",p),!cv_cli_excluded(i).
.violation ClassVarCliWarning(at: span,ordinal: number,text: symbol) code "CV_CLI_WARNING" "ClassVar CLI warning"
ClassVarCliWarning(@place("",0),i,cat("Warning: ",p," not found\n")) :- option("entryPoint","_classvar_cli_prepare"),option_item("cvCliPaths",i,p),option_item("cvCliKinds",i,"missing").
.decl cv_cli_issue(i: number,file: symbol,line: symbol,column: symbol,code: symbol,message: symbol)
cv_cli_issue(i,f,l,c,k,m) :- option_item("cvCliFiles",i,f),option_item("cvCliLines",i,l),option_item("cvCliColumns",i,c),option_item("cvCliCodes",i,k),option_item("cvCliMessages",i,m).
.decl cv_cli_before(i: number,j: number)
cv_cli_before(i,j) :- cv_cli_issue(i,f,l,_,_,_),cv_cli_issue(j,g,m,_,_,_),(@str_cmp(f,g)<0;f=g,to_number(l)<to_number(m);f=g,to_number(l)=to_number(m),i<j).
.decl cv_cli_count(n: number)
cv_cli_count(n) :- option("entryPoint","_classvar_cli_report"),n=count:{cv_cli_issue(_,_,_,_,_,_)}.
.violation ClassVarCliOutput(at: span,phase: number,ordinal: number,text: symbol) code "CV_CLI_OUTPUT" "ClassVar CLI output"
ClassVarCliOutput(@place("",0),0,0,"No ClassVar check issues found.\n") :- cv_cli_count(0).
ClassVarCliOutput(@place("",0),0,0,cat("Found ",to_string(n)," ClassVar check issue(s):\n\n")) :- cv_cli_count(n),n>0.
ClassVarCliOutput(@place("",0),1,rank,cat(f,":",l,":",c,": [",k,"] ",m,"\n")) :- option("entryPoint","_classvar_cli_report"),cv_cli_issue(i,f,l,c,k,m),rank=count:{cv_cli_before(j,i)}.
.violation ClassVarCliExit(at: span,status: number) code "CV_CLI_EXIT" "ClassVar CLI status"
ClassVarCliExit(@place("",0),1) :- cv_cli_count(n),n>0,option("cvCliCheck","1").
ClassVarCliExit(@place("",0),0) :- cv_cli_count(n),(n=0;option("cvCliCheck","0")).
'''
    return text
