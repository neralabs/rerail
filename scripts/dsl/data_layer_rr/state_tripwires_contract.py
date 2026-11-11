"""Fresh State Tripwires contracts for API scopes, diagnostics, and deduplication."""
import inspect
from pathlib import Path

from contract_runner import main
from dto import canonical
from state_tripwires_port import scan

CHECK='serverplan_state_tripwires'


def cases(_module):
    result=[]
    signature='class Step(ServerPlanStep):\n    def __init__(self, table_name: str, count: int):\n        pass\n'
    for index,path in enumerate(['impl/common/l2070_server_plan/a.py','outside/a.py','outside/tmp/test_a.py',
                                 'outside/tests/a.py','tests/a.py','a.py']):
        result.append(dict(id=f'signature-scope-{index}',kind='source',path=path,source=signature))
    for index,ir in enumerate(['CommitFinalizeRuntimeIR(x=1)','ns.CommitFinalizeRuntimeIR(x=1)',
                               'None',"{'x':1}","(other if ready else alternate)"]):
        result.append(dict(id=f'ir-display-{index}',kind='source',path='impl/common/l2070_server_plan/a.py',
            source=f'def compile_step():\n    return CompiledStep(ir={ir})\n'))
    result.append(dict(id='same-line-queue-dedup',kind='source',path='outside/a.py',
        source='def f():\n    set_queue_auth_token(); set_queue_auth_token()\n'))
    result.append(dict(id='nested-call-qualname',kind='source',path='outside/a.py',
        source='class C:\n    def f(self):\n        def inner():\n            obj.set_queue_auth_token()\n'))
    result.append(dict(id='first-call-before-shortest-path',kind='graph',
        path='impl/common/l2070_server_plan/a.py',source='''def _sql(conn):
    return conn.execute("select 1")
def _long(conn):
    return _sql(conn)
def _short(conn):
    return conn.execute("select 2")
def compile(conn):
    _long(conn)
    return _short(conn)
'''))
    result.append(dict(id='cycle-open-negative-memo',kind='graph',
        path='impl/common/l2070_server_plan/a.py',source='''def a_lazy(conn):
    b_lazy(conn)
    return _sql(conn)
def b_lazy(conn):
    return a_lazy(conn)
def _sql(conn):
    return conn.execute("select 1")
'''))
    result.append(dict(id='branded-signature-clean',kind='source',path='outside/tests/a.py',
        source='class Step(ServerPlanStep):\n    def __init__(self, table_name: SqlTableName, count: RowCount):\n        pass\n'))
    result.append(dict(id='allowlisted-ir-clean',kind='source',path='impl/common/l2070_server_plan/a.py',
        source='def compile_step():\n    return CompiledStep(ir=SqlStepIR(sql=statement))\n'))
    result.append(dict(id='queue-call-in-test-clean',kind='source',path='outside/tests/a.py',
        source='def f():\n    set_queue_auth_token()\n'))
    return result


def exercise(module,case,_scratch):
    arguments=dict(path=Path(case['path']),source=case['source'])
    return dict(value=canonical(module.scan_source(**arguments)))


def install(backend,module,*,detectors):
    for entry in ['scan_source','scan_tree','scan_project_sources']:
        signature=inspect.signature(getattr(module,entry))
        def wrapper(entry,signature):
            def run(*args,**kwargs):
                return scan(backend,module,signature.bind(*args,**kwargs).arguments,entry)
            return run
        backend.replace(module,entry,wrapper(entry,signature))


if __name__=='__main__':
    main(CHECK,'rerail.state-tripwires-source-contract/v1',cases,exercise,install,
         ['state_tripwires_contract.py','state_tripwires_port.py'])
