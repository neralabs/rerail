"""Transport caller syntax and native plan-scope collector state; make no detector decisions."""
import ast
import json
from pathlib import Path
from ast_transport import columns
CHECK='plan_scope_side_effects'


def state(module,rows,names):
    values=[r['parameters'] for r in rows if r['code']=='PSE_STATE']
    if len(values)!=1:raise ValueError('One native plan-scope state required')
    p=values[0];eager={r['parameters']['name'] for r in rows if r['code']=='PSE_READ_NAME'}
    return [module._PlanScopeState(names,bool(p['write']),eager)] if p['present'] else []


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_pse_configuration',[])
    if not rows or any(r['code']!='PSE_CONFIG'for r in rows):raise ValueError('Missing native plan-scope configuration')
    for row in rows:
        p=row['parameters'];value=json.loads(p['value'])
        if p['key']=='extractor':
            for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:value[key]=tuple(value[key])
            backend.replace(module,'PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR',type(module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR)(**value))
        else:backend.replace(module,p['key'],frozenset(value))
    def helper(name,node,scope_names=(),write=False,eager=()):
        rows=backend.native(CHECK,'_pse_helper',[],dict(columns(node),pseGraph='1',pseHelper=name,
            pseSeedNames=sorted(scope_names),pseSeedWrite=str(int(write)),pseSeedEagerNames=sorted(eager)))
        if any(r['code'] not in {'PSE_HELPER_VALUE','PSE_HELPER_KIND','PSE_HELPER_NAME'}for r in rows):raise ValueError('Unexpected plan-scope helper response')
        values=[r['parameters']['value_json']for r in rows if r['code']=='PSE_HELPER_VALUE']
        if values:
            if len(values)!=1 or len(rows)!=1:raise ValueError('One native scalar helper result required')
            return json.loads(values[0])
        kinds=[r['parameters']['kind']for r in rows if r['code']=='PSE_HELPER_KIND']
        if len(kinds)!=1:raise ValueError('One native collection helper kind required')
        values=[r['parameters']['name']for r in sorted((r for r in rows if r['code']=='PSE_HELPER_NAME'),key=lambda r:r['parameters']['ordinal'])]
        return set(values)if kinds[0]=='set'else values
    for name in ['_call_name','_receiver_tokens','_is_plan_scope_item','_is_write_plan_scope_item','_scope_name','_assigned_names']:
        backend.replace(module,name,lambda node,_name=name:helper(_name,node))
    backend.replace(module,'_referenced_names',lambda node,context:helper('_referenced_names',node))
    backend.replace(module,'_is_allowed_scope_execute',lambda node,scope_names:helper('_is_allowed_scope_execute',node,scope_names))
    initial_states={}
    class Collector(module.AstEventCollector):
        def __init__(self,rel_path,scope_names,*,is_write_plan):
            self.rel_path=rel_path;self.violations=[];self._root=None;self._seed_names=scope_names;self._seed_write=is_write_plan
            seed=(tuple(sorted(scope_names)),is_write_plan)
            if seed not in initial_states:
                initial_states[seed]=backend.native(CHECK,'_pse_initial',[],dict(pseSeedNames=list(seed[0]),pseSeedWrite=str(int(is_write_plan))))
            self._scope_stack=state(module,initial_states[seed],scope_names)
        @property
        def _current_scope(self):return self._scope_stack[-1] if self._scope_stack else None
        def start_file(self,path,source,context):self.path=path;self.source=source
        def on_node(self,node,context):
            if self._root is not None:return
            current=node
            while (parent:=context.parent(current)) is not None:current=parent
            self._root=current
        def _is_typed_eager_read_call(self,node):return helper('_is_typed_eager_read_call',node)
        def _looks_like_python_side_effect(self,node):return helper('_looks_like_python_side_effect',node)
        def _branches_on_eager_read(self,node,context):
            current=self._current_scope
            return helper('_branches_on_eager_read',node,()if current is None else current.scope_names,
                False if current is None else current.is_write_plan,()if current is None else current.eager_read_names)
        def finish_file(self):
            tree=self._root if self._root is not None else ast.Module(body=[],type_ignores=[])
            rows=backend.native(CHECK,'_pse_collect',[],dict(columns(tree),pseGraph='1',psePath=self.rel_path,
                pseSeedNames=sorted(self._seed_names),pseSeedWrite=str(int(self._seed_write))))
            if any(r['code'] not in {'PSE_FACT','PSE_STATE','PSE_READ_NAME'}for r in rows):raise ValueError('Unexpected native plan-scope collector response')
            facts=[dict(code=p['source_code'],file_path=p['file_path'],line_number=p['line_number'],message=p['source_message'],severity=p['severity'])
                for r in sorted((r for r in rows if r['code']=='PSE_FACT'),key=lambda r:r['parameters']['ordinal'])for p in [r['parameters']]]
            self.violations=[module.Violation(**p)for p in facts];self._scope_stack=state(module,rows,self._seed_names)
            return facts
    backend.replace(module,'FileCollector',Collector)
    def registry(root):
        def extract(path,source):
            relative=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
            return Collector(relative,set(),is_write_plan=False)
        return {module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id:extract}
    backend.replace(module,'plan_scope_side_effects_fact_extractors',registry)
