"""Generate bounded native JSON projection stages and passive DTO declarations."""
import ast
import hashlib
from pathlib import Path


MODELS={'ServerPlanClassFacts','ServerPlanFunctionFacts','ServerPlanCompileMethodFacts',
        'GrammarSurfaceHit','ServerPlanFileFacts','ProjectServerPlanFacts'}


def dto_declarations(source):
    path=source/'scripts/lint_lib/serverplan_facts.py';nodes=[]
    for node in ast.parse(path.read_text()).body:
        if isinstance(node,ast.ClassDef) and node.name in MODELS:
            node.body=[v for v in node.body if isinstance(v,(ast.AnnAssign,ast.Assign,ast.Expr))];nodes.append(node)
        elif isinstance(node,ast.Assign) and isinstance(node.value,ast.Call) and isinstance(node.value.func,ast.Name) and node.value.func.id=='NewType':nodes.append(node)
    assert {n.name for n in nodes if isinstance(n,ast.ClassDef)}==MODELS
    return ('"""Passive field declarations from pinned source. Source methods are not copied."""\n'
        '# Source SHA256: '+hashlib.sha256(path.read_bytes()).hexdigest()+'\n'
        'from typing import NewType\nfrom pydantic import BaseModel, ConfigDict\n\n'+
        '\n\n'.join(ast.unparse(n) for n in nodes)+'\n')


def projection_library():
    text='''// Generated bounded projection: root -> model tuple -> model -> primitive tuple.
// Primitive casts are native. Frontends only wrap typed values in passive DTOs.
.decl mp_boolean(b: number, node: number, value: symbol)
mp_boolean(b,n,v) :- sv_kind(b,n,"boolean"), sv_raw(b,n,v).
mp_boolean(b,n,"true") :- sv_kind(b,n,"number"), sv_integer(b,n,"1").
mp_boolean(b,n,"false") :- sv_kind(b,n,"number"), sv_integer(b,n,"0").
mp_boolean(b,n,"true") :- sv_kind(b,n,"string"), sv_text(b,n,v), match("(?i:1|on|t|true|y|yes)",v).
mp_boolean(b,n,"false") :- sv_kind(b,n,"string"), sv_text(b,n,v), match("(?i:0|off|f|false|n|no)",v).
.decl mp_scalar(b: number, node: number, value: symbol)
mp_scalar(b,n,v) :- sv_check(b,n,_,_,_,_,"int",_,_), sv_integer(b,n,v).
mp_scalar(b,n,v) :- sv_check(b,n,_,_,_,_,"bool",_,_), mp_boolean(b,n,v).
.decl mp_json0(b: number, node: number, value: symbol)
mp_json0(b,n,v) :- mp_scalar(b,n,v).
mp_json0(b,n,v) :- sv_raw(b,n,v), !mp_scalar(b,n,_).
'''
    for level in range(1,5):
        prior=f'mp_json{level-1}';relation=f'mp_json{level}'
        text+=f'''.decl {relation}(b: number, node: number, value: symbol)
{relation}(b,n,v) :- mp_scalar(b,n,v).
{relation}(b,n,v) :- sv_raw(b,n,v), !sv_kind(b,n,"object"), !sv_kind(b,n,"array"), !mp_scalar(b,n,_).
{relation}(b,n,cat("[",body,"]")) :- sv_kind(b,n,"array"), body = join v by i sep "," : {{ sv_node(b,c,n,_,i), {prior}(b,c,v) }}.
{relation}(b,n,cat("{{",body,"}}")) :- sv_kind(b,n,"object"), body = join pair by i sep "," : {{ sv_node(b,c,n,k,i), {prior}(b,c,v), pair = cat(@json_str(k),":",v) }}.
'''
    return text


POLICY='''#include "data-layer/model-projection.rr"
sv_root(b) :- option("entryPoint","_project_facts"), sv_kind(b,0,"object").
sv_root(b) :- option("entryPoint","_project_messages"), sv_kind(b,0,"object").
.violation ProjectedModel(at: span, bundle: number, path_order: symbol, normalized_json: symbol) code "SF_MODEL" "projected model"
ProjectedModel(@place("",0),b,p,v) :- option("entryPoint","_project_facts"), ss_file(b,p), mp_json4(b,0,v).
.violation ProjectedMessage(at: span, bundle: number, ordinal: number, value: symbol) code "SPG_MESSAGE" "message"
ProjectedMessage(@place("",0),b,i,cat(p,": ",message)) :- option("entryPoint","_project_messages"), ss_event(b,i,_,_,message), i = -1, sf_path(b,i,p).
ProjectedMessage(@place("",0),b,i,cat(p,":",line,": ",message)) :- option("entryPoint","_project_messages"), ss_event(b,i,line,_,message), i >= 0, sf_path(b,i,p).
'''


def generate(root,source):
    # Keep one authored definition. This generated API policy is regenerated
    # from the detector policy and has its own identity and input contracts.
    base=(root/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-grammar-surface.rr').read_text()
    import re
    api,count=re.subn(r'(?m)^\.policy .*$', '.policy "data-layer-serverplan-grammar-surface-project" revision 1',base)
    assert count==1
    outputs={
        'scripts/dsl/data_layer_rr/model_dtos.py':dto_declarations(source),
        'docs/rule-syntax/lib/data-layer/model-projection.rr':projection_library(),
        'gate/data-layer/packs/data-layer/candidates/rr/serverplan-grammar-surface-project.rr':api+POLICY}
    for name,text in outputs.items():(root/name).write_text(text)
    return outputs


if __name__=='__main__':
    import sys
    root=Path(__file__).resolve().parents[3];sys.path.insert(0,str(root/'scripts/dsl/data_layer_rr'))
    from migration import PIN,git
    source=Path(sys.argv[1]).absolute();assert git(source,'rev-parse','HEAD')==PIN
    generate(root,source)
