"""Record callback-visible scope, findings, mutation, and repeated finish semantics."""
import ast
from pathlib import Path
from contract_runner import main
from dto import canonical
from side_effects_collector_contract import drive
from side_effects_helper_contract import context
CHECK='plan_scope_side_effects'
TEXT='with open_write_plan() as p:\n n=q.count()\n if n: lazy.resolve()\n'


def cases(_module):
    return [dict(kind=kind,id=kind,source=TEXT)for kind in ['callbacks','private-with','mutate-after-callbacks','clear-after-finish','reuse-file']]


def exercise(module,case,scratch):
    try:
        path=Path('caller.py');tree=ast.parse(case['source']);collector=module.FileCollector('caller.py',set(),is_write_plan=False)
        if case['kind']in {'callbacks','private-with'}:
            ctx=context(tree);w=tree.body[0];assignment=w.body[0];branch=w.body[1];call=branch.body[0].value
            events=[('_enter_with_body',w,None),('on_Assign',assignment,ctx),('on_If',branch,ctx),('on_Call',call,ctx),('_exit_with_body',w,None)]if case['kind']=='private-with'else[
                ('on_With',w,ctx),('on_Assign',assignment,ctx),('on_If',branch,ctx),('on_Call',call,ctx),('on_exit_With',w,ctx)]
            trace=[]
            for name,node,ctx in events:
                result=getattr(collector,name)(node)if ctx is None else getattr(collector,name)(node,ctx)
                trace.append(dict(event=name,result=result,stack=canonical(collector._scope_stack),current=canonical(collector._current_scope),violations=canonical(collector.violations)))
            return dict(trace=trace,facts=canonical(collector.finish_file()))
        drive(collector,tree,path,case['source'])
        if case['kind']=='mutate-after-callbacks':
            before=canonical(collector.violations);tree.body[0].body[1].body[0].value.func.attr='clean'
            return dict(before=before,facts=canonical(collector.finish_file()),after=canonical(collector.violations))
        first=collector.finish_file()
        if case['kind']=='clear-after-finish':collector.violations.clear()
        else:drive(collector,ast.parse('with open_read_plan():\n other.resolve()\n'),Path('second.py'),'other caller source')
        return dict(first=canonical(first),second=canonical(collector.finish_file()),violations=canonical(collector.violations))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args)))


def install(backend,module,detectors=False):
    from side_effects_port import install as collector
    collector(backend,module,detectors)

if __name__=='__main__':main(CHECK,'rerail.side-effects-incremental/v1',cases,exercise,install,['side_effects_incremental_contract.py','side_effects_port.py','side_effects_collector_contract.py','side_effects_helper_contract.py','ast_transport.py'])
