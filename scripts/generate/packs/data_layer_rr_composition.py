"""Share native composition source analysis between production and explicit-source APIs."""
import json
import re
from data_layer_rr_text import module


def generate(source,target,header):
    original=module(source,'plan_composition_architecture')
    text=header('plan_composition_architecture',target)
    for relation,name in [('spc_import_part','_FORBIDDEN_IMPORT_PARTS'),('spc_runtime_type','_FORBIDDEN_RUNTIME_TYPE_PARTS'),
                          ('spc_terminal','_FORBIDDEN_BUILDER_CALLS'),('spc_mutable','_MUTABLE_RETURN_ROOTS')]:
        text+=f'.decl {relation}(value: symbol)\n'
        text+=''.join(f'{relation}({json.dumps(value)}).\n' for value in sorted(getattr(original,name)))
    text+='#include "data-layer/composition-source.rr"\n'
    from data_layer_rr_cli import api_scope
    api=api_scope(text)
    api=re.sub(r'(?m)^\.policy .*$', '.policy "data-layer-plan-composition-architecture-source" revision 1',api)
    # Project behavior keeps its explicit producer contract. The source API never runs probes.
    text+='''
.artifact "plan_composition_probes" inputs "**/*.py"
.decl spc_fingerprint(fp: symbol)
.input spc_fingerprint(IO=json,artifact="plan_composition_probes",path="/fingerprints")
.decl spc_first_ref(ref: symbol)
.input spc_first_ref(IO=json,artifact="plan_composition_probes",path="/first_refs")
.decl spc_second_ref(ref: symbol)
.input spc_second_ref(IO=json,artifact="plan_composition_probes",path="/second_refs")
.violation CompositionFingerprint(at: span) code "SPC-012" "equivalent semantic plans produced different fingerprints"
CompositionFingerprint(@place("l2070_batching/semantic_plan.py",1)) :- spc_fingerprint(a),spc_fingerprint(b),a!=b.
.violation CompositionRefs(at: span) code "SPC-005" "colliding fragment imports reused destination refs"
CompositionRefs(@place("l2070_batching/semantic_plan.py",1)) :- spc_first_ref(r),spc_second_ref(r).
'''
    return text,api
