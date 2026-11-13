"""Compare native tape fact rows with the same pinned collector corpus, including positives."""
from pathlib import Path
from types import SimpleNamespace
from contract_runner import main
from side_effects_collector_contract import cases as all_cases,exercise as source_exercise
from dto import canonical
CHECK='plan_scope_side_effects'

def cases(module):return [r for r in all_cases(module)if r['kind']=='extract']

def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_pse_source'):return source_exercise(module,case,scratch)
    try:
        result=module._rerail_pse_source('impl/a.py',case['source'])
        return dict(value=canonical(dict(facts=result.facts,errors=result.errors),scratch.resolve()))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch.resolve())))

def install(backend,module,detectors=False):
    from side_effects_port import install as configure
    configure(backend,module)
    def collect(path,source):
        rows=backend.native(CHECK,'_pse_source',[dict(path=path,source=source)],dict(psePath=path))
        if any(r['code'] not in {'PSE_FACT','PSE_PARSE'}for r in rows):raise ValueError('Unexpected native source collector response')
        statuses=[r['parameters']for r in rows if r['code']=='PSE_PARSE']
        if len(statuses)!=1:raise ValueError('One native source parse status required')
        facts=[dict(code=p['source_code'],file_path=p['file_path'],line_number=p['line_number'],message=p['source_message'],severity=p['severity'])for r in sorted((r for r in rows if r['code']=='PSE_FACT'),key=lambda r:r['parameters']['ordinal'])for p in [r['parameters']]]
        family=module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id;status=statuses[0]
        return SimpleNamespace(facts={family:facts}if status['present']else {},errors={}if status['present']else {family:status['detail']})
    module._rerail_pse_source=collect

if __name__=='__main__':main(CHECK,'rerail.side-effects-tape/v1',cases,exercise,install,['side_effects_tape_contract.py','side_effects_collector_contract.py','side_effects_port.py','ast_transport.py'])
