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
    add('pending-provenance','def f(executor):\n executor._conn\n',module._BASE_SERVER_PLAN_FILE)
    return rows


def exercise(module,case,_temporary):
    try:return dict(value=canonical(module.scan_source(Path(case['path']),case['source'])))
    except BaseException as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args)))


if __name__=='__main__':
    from contract_runner import main
    from dispatch_port import install
    main(CHECK,'rerail.dispatch-source-contract/v1',cases,exercise,install,
         ['dispatch_contract.py','dispatch_port.py','contract_runner.py'])
