"""Transport caller syntax and native plan-scope collector state; make no detector decisions."""
import json
from ast_transport import columns, context_columns
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
    def helper(name,node,scope_names=(),write=False,eager=(),context=None):
        rows=backend.native(CHECK,'_pse_helper',[],dict(columns(node),pseGraph='1',pseHelper=name,
            pseSeedNames=sorted(scope_names),pseSeedWrite=str(int(write)),pseSeedEagerNames=sorted(eager),
            **({}if context is None else dict(context_columns(node,context),pseUseContext='1'))))
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
    backend.replace(module,'_referenced_names',lambda node,context:helper('_referenced_names',node,context=context))
    backend.replace(module,'_is_allowed_scope_execute',lambda node,scope_names:helper('_is_allowed_scope_execute',node,scope_names))
    initial_states={}
    class Collector(module.AstEventCollector):
        def __init__(self,rel_path,scope_names,*,is_write_plan):
            self.rel_path=rel_path;self.__violations=[];self.__pending=[]
            seed=(tuple(sorted(scope_names)),is_write_plan)
            if seed not in initial_states:
                initial_states[seed]=backend.native(CHECK,'_pse_initial',[],dict(pseSeedNames=list(seed[0]),pseSeedWrite=str(int(is_write_plan))))
            self.__stack=state(module,initial_states[seed],scope_names)
            # A seed frame already exposes the caller's mutable alias set.
            self.__visible=bool(self.__stack)
        @property
        def _scope_stack(self):
            self.__flush();self.__visible=True;return self.__stack
        @_scope_stack.setter
        def _scope_stack(self,value):
            self.__flush();self.__visible=True;self.__stack=value
        @property
        def violations(self):
            self.__flush();self.__visible=True;return self.__violations
        @violations.setter
        def violations(self,value):
            self.__flush();self.__visible=True;self.__violations=value
        @property
        def _current_scope(self):return self._scope_stack[-1] if self._scope_stack else None
        def __queue(self,method,node,context=None):
            # Bodies get their own callbacks. Capture each header's syntax now,
            # before the caller can mutate it; never copy an entire enclosing body.
            data=columns(node,omit_root_fields=('body','orelse')if method in {'enter','exit','if'}else ())
            references=context_columns(node.test,context)if method=='if'else None
            self.__pending.append((method,data,references,self.rel_path))
            # Exit can raise synchronously. An escaped mutable state/list also
            # requires synchronous updates, so held references remain live.
            if self.__visible or method=='exit':self.__flush()
        def __flush(self):
            if not self.__pending:return
            pending,self.__pending=self.__pending,[]
            count=len(self.__stack)
            options=dict(pseGraph='1',pseFrameCount=str(count),pseFrameWrites=[str(int(s.is_write_plan))for s in self.__stack],
                pseFrameNameFrames=[],pseFrameNames=[],pseFrameReadFrames=[],pseFrameReads=[],
                pseEventMethods=[],pseEventRoots=[],pseEventPaths=[],pseContextEvents=[],pseContextKinds=[],pseContextIds=[])
            for i,frame in enumerate(self.__stack):
                for name in sorted(frame.scope_names):options['pseFrameNameFrames'].append(str(i-count));options['pseFrameNames'].append(name)
                for name in sorted(frame.eager_read_names):options['pseFrameReadFrames'].append(str(i-count));options['pseFrameReads'].append(name)
            for event,(method,data,references,path)in enumerate(pending):
                offset=len(options.get('astKinds',[]));options['pseEventRoots'].append(str(offset));options['pseEventMethods'].append(method);options['pseEventPaths'].append(path)
                for key,values in data.items():
                    if key=='astParents':values=[str(int(v)+offset)if v!='-1'else v for v in values]
                    options.setdefault(key,[]).extend(values)
                if references is not None:
                    options['pseContextEvents'].extend([str(event)]*len(references['astContextKinds']))
                    options['pseContextKinds'].extend(references['astContextKinds']);options['pseContextIds'].extend(references['astContextIds'])
            rows=backend.native(CHECK,'_pse_events',[],options)
            allowed={'PSE_EVENT_PUSH','PSE_EVENT_POP','PSE_EVENT_NAME','PSE_EVENT_READ','PSE_EVENT_FACT'}
            if any(r['code']not in allowed for r in rows):raise ValueError('Unexpected native collector operation')
            groups={}
            for row in rows:groups.setdefault(row['parameters']['event'],[]).append(row)
            for event,items in sorted(groups.items()):
                names={r['parameters']['name']for r in items if r['code']=='PSE_EVENT_NAME'}
                for row in items:
                    p=row['parameters'];code=row['code']
                    if code=='PSE_EVENT_PUSH':self.__stack.append(module._PlanScopeState(names,bool(p['write']),set()))
                    elif code=='PSE_EVENT_POP':self.__stack.pop()
                    elif code=='PSE_EVENT_READ':self.__stack[-1].eager_read_names.add(p['name'])
                    elif code=='PSE_EVENT_FACT':self.__violations.append(module.Violation(p['source_code'],p['file_path'],p['line_number'],p['source_message'],p['severity']))
        def _enter_with_body(self,node):self.__queue('enter',node)
        def _exit_with_body(self,node):self.__queue('exit',node)
        def on_With(self,node,context):self._enter_with_body(node)
        def on_exit_With(self,node,context):self._exit_with_body(node)
        def on_AsyncWith(self,node,context):self._enter_with_body(node)
        def on_exit_AsyncWith(self,node,context):self._exit_with_body(node)
        def on_Assign(self,node,context):self.__queue('assign',node)
        def on_AnnAssign(self,node,context):self.__queue('assign',node)
        def on_If(self,node,context):self.__queue('if',node,context)
        def on_Attribute(self,node,context):self.__queue('attribute',node)
        def on_Call(self,node,context):self.__queue('call',node)
        def _is_typed_eager_read_call(self,node):return helper('_is_typed_eager_read_call',node)
        def _looks_like_python_side_effect(self,node):return helper('_looks_like_python_side_effect',node)
        def _branches_on_eager_read(self,node,context):
            current=self._current_scope
            return helper('_branches_on_eager_read',node,()if current is None else current.scope_names,
                False if current is None else current.is_write_plan,()if current is None else current.eager_read_names,context=context)
        def finish_file(self):
            self.__flush()
            return module.require_json_fact([dict(code=v.code,file_path=v.file_path,line_number=v.line_number,message=v.message,severity=v.severity)for v in self.__violations])
    backend.replace(module,'FileCollector',Collector)
    def registry(root):
        def extract(path,source):
            relative=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
            return Collector(relative,set(),is_write_plan=False)
        return {module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id:extract}
    backend.replace(module,'plan_scope_side_effects_fact_extractors',registry)
