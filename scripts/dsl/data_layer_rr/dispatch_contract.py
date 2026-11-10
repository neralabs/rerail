"""Record dispatch positives, complete DTOs, occurrence ordering and unresolved provenance."""
from pathlib import Path
from dto import canonical
CHECK='serverplan_dispatch_architecture'
DEFAULT='impl/common/l2070_server_plan/optimizers/example.py'


def cases(module):
    rows=[]
    def add(name,source,path=DEFAULT):rows.append(dict(id=name,source=source,path=path))
    for name in sorted(module._BACKEND_FAMILY_ATTRS):
        add('family-'+name,f'ctx.{name}\n')
    for name in sorted(module._BACKEND_CALLS):
        add('backend-call-'+name,f'{name}() == "sqlite"\n')
    for i,value in enumerate(['sqlite','SQLITE','poﬆgres','poſtgres','PoStGrEsQl','İSQLITE','portable']):
        add('literal-'+str(i),f'ctx.backend_type == {value!r}\n')
    for t in ['PhysicalStatementGroup','ConditionalStep','AIntent','AStep','AStrategy','CompiledStep','IntentStep','ServerPlanStep','str']:
        for i,template in enumerate(['isinstance(value, {t})','type(value) is {t}','type(value).__name__ == "{t}"',
                                    'value.__class__.__name__ == "{t}"','isinstance(value, ({t}, str))','isinstance(value, ({t} | str))']):
            add('step-'+t+'-'+str(i),template.format(t=t)+'\n')
    for i,test in enumerate(['dialect','not dialect','bool(dialect)','ctx.dialect','ctx.dialect.supports_returning',
        'flag and dialect','not (flag and dialect)','getattr(ctx,"dialect",None)','getattr(ctx,"dialect",None) is None']):
        for label,template in [('if','if {x}: pass\n'),('while','while {x}: pass\n'),('assert','assert {x}\n'),('ternary','value=1 if {x} else 2\n')]:
            add('dialect-'+str(i)+'-'+label,template.format(x=test))
    for label,source in [
        ('backend-none','backend_type is None\n'),('comparison-chain','ctx.backend_type == "sqlite" == "mysql"\n'),
        ('nested-scopes','class C:\n def f(self):\n  def inner():\n   ctx.is_sql_backend\n'),
        ('duplicates','ctx.is_sql_backend; ctx.is_sql_backend\nctx.is_sql_backend\n'),
        ('whitespace-occurrences','ctx.backend_type=="sqlite"\nctx.backend_type == "sqlite"\nctx.backend_type=="sqlite"\n'),
        ('class-pattern','match value:\n case PhysicalStatementGroup(): pass\n'),
        ('qualified-pattern','match value:\n case model.PhysicalStatementGroup(): pass\n'),
        ('guard','match value:\n case x if dialect: pass\n'),
        ('decorator','@deco(ctx.is_sql_backend)\ndef f(): pass\n'),
        ('unicode-offset','é="😀"; ctx.backend_type == "sqlite"\n'),
        ('long-expression','ctx.backend_type == ("sqlite" # '+('x'*320)+'\n)\n'),
        ('clean','pass\n'),('syntax','def broken(:\n')]:add(label,source)
    for i,path in enumerate(['other/a.py','impl/common/l2070_server_plan/backend_profiles.py','l2070_batching/a.py',
        'impl/common/l2070_server_plan/steps/tests/a.py']):add('scope-'+str(i),'ctx.is_sql_backend\n',path)
    add('executor-connection','def f(executor):\n executor._conn\n',module._BASE_SERVER_PLAN_FILE)
    base=module._BASE_SERVER_PLAN_FILE
    def executor(name,body,parameters='backend, rendering'):
        add('executor-'+name,'def run('+parameters+'):\n'+''.join(' '+line+'\n' for line in body.splitlines()),base)
    for name,body in [
        ('direct','backend.transport'),('approved','backend.get_lowering_context().branch_table'),
        ('alias','selected=backend\nselected.transport'),
        ('kill','selected=backend\nselected=rendering\nselected.transport'),
        ('restore','selected=rendering\nselected=backend\nselected.transport'),
        ('prebind','selected.transport\nselected=backend'),
        ('future-multi','selected.transport\nselected=backend\nselected=rendering'),
        ('unpack-owner','selected,other=backend,rendering\nselected.transport'),
        ('unpack-clean','other,selected=backend,rendering\nselected.transport'),
        ('nested-unpack','(selected,(other,last))=(backend,(rendering,rendering))\nselected.transport'),
        ('loop','for selected in (backend,):\n selected.transport'),
        ('loop-clean','for selected in (rendering,):\n selected.transport'),
        ('comprehension','[selected.transport for selected in (backend,)]'),
        ('match','match backend:\n case selected:\n  selected.transport'),
        ('match-star','match backend:\n case [*selected]:\n  selected.transport'),
        ('match-mapping-rest','match backend:\n case {**selected}:\n  selected.transport'),
        ('named-expression','(selected:=backend).transport'),
        ('attribute-store','self.engine=backend\nself.engine.transport'),
        ('subscript-store','mapping["engine"]=backend'),
        ('annotation-only','selected: BackendExecutor\nselected.transport'),
        ('annotation-value','selected: BackendExecutor=rendering\nselected.transport'),
        ('yield','yield backend'),('yield-from','yield from backend'),
        ('return','return backend'),('return-tuple','return backend, rendering'),
        ('reflect-dynamic','getattr(backend,name)'),('reflect-allowed','getattr(backend,"get_lowering_context")'),
        ('reflect-vars','vars(backend)'),('attrgetter','attrgetter("transport")(backend)'),
        ('attrgetter-multi','attrgetter("get_lowering_context","transport")(backend)'),
        ('getattribute','object.__getattribute__(backend,"transport")'),
        ('consumer','consume(backend)'),('keyword-consumer','consume(None,executor=backend)'),
        ('keyword-only-consumer','consume(executor=backend)'),('attribute-consumer','service.consume(backend)'),
        ('comparison','backend is rendering'),('branch','if flag:\n selected=backend\nelse:\n selected=rendering\nselected.transport'),
        ('deletion','selected=backend\ndel selected\nselected.transport'),
        ('unary','(~backend).transport'),('await','(await backend).transport'),
    ]:
        if name=='await':add('executor-await','async def run(backend):\n (await backend).transport\n',base)
        else:executor(name,body)
    for annotation in ['BackendExecutor','"BackendExecutor"','Optional[BackendExecutor]','Awaitable[BackendExecutor]',
        'Annotated[BackendExecutor,"x"]','Coroutine[Any,Any,BackendExecutor]','BackendExecutor|None',
        '"None | BackendExecutor"','" Optional[BackendExecutor] "','ServerPlanBackendProfile','PlanExecutor']:
        executor('annotation-'+annotation,'engine.transport',parameters='engine: '+annotation)
    for container in ['[backend,rendering]','(backend,rendering)','{"owner":backend,"other":rendering}',
        '{1:backend,2:rendering}','{True:backend,False:rendering}','{1.0:backend,2.0:rendering}',
        '{"owner":rendering,"owner":backend}']:
        for index in ['0','1','True','False','-1','2','1.0','"owner"','"other"','"missing"']:
            executor('selection-'+container+'-'+index,'selected='+container+'['+index+']\nselected.transport')
    for name,source in [
        ('import-alias','from somewhere import BackendExecutor as Engine\ndef f(engine:Engine):\n engine.transport\n'),
        ('type-alias','Alias=BackendExecutor\ndef f(engine:Alias):\n engine.transport\n'),
        ('subtype','class CustomBackendExecutor(BackendExecutor): pass\ndef f(engine:CustomBackendExecutor):\n engine.transport\n'),
        ('factory','def make()->BackendExecutor: pass\ndef f():\n make().transport\n'),
        ('closure','def f(backend):\n def g():\n  backend.transport\n'),
        ('closure-shadow','def f(backend):\n def g(backend:Rendering):\n  backend.transport\n'),
        ('factory-scope','def _create_backend_executor():\n executor=make_backend_executor()\n if executor is not None:\n  return executor\n'),
        ('module-alias','engine=make_backend_executor()\nengine.transport\n'),
        ('module-kill','engine=make_backend_executor()\nengine=rendering\nengine.transport\n'),
        ('base-invalid','def broken(:\n'),
    ]:add('executor-'+name,source,base)
    return rows


def exercise(module,case,_temporary):
    try:return dict(value=canonical(module.scan_source(Path(case['path']),case['source'])))
    except BaseException as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args)))


if __name__=='__main__':
    from contract_runner import main
    from dispatch_port import install
    main(CHECK,'rerail.dispatch-source-contract/v1',cases,exercise,install,
         ['dispatch_contract.py','dispatch_port.py','contract_runner.py'])
