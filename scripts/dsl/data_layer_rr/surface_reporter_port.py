"""Transport supplied model DTOs; native relations validate and report them."""
import json
from pathlib import Path
from first_ten_production_contract import normalize
from migration import ROOT, compare
from replay import execute_workspace
from model_transport import encode_models


def native_case(engine,case,policy=None):
    policy=policy or ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-grammar-surface.rr'
    execution=execute_workspace(engine,policy,[dict(path='reporter-facts.json',
        source=json.dumps(dict(bundles=encode_models(case['bundles'])),ensure_ascii=False))],dict(entryPoint='_report_facts'))
    rows=execution['diagnostics'];roots=[r['parameters']['root'] for r in rows if r['code']=='SF_ROOT']
    compare(len(roots),1)
    from pydantic import ValidationError
    try:value=dict(findings=report_rows(rows,lambda **fields:fields))
    except ValidationError as error:
        value=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=list(error.args),
            details=error.errors(include_url=False,include_context=False)))
        compare(normalize(value['error'],roots[0]),normalize(json.loads(next(
            r['parameters']['error_json'] for r in rows if r['code']=='SF_ERROR')),roots[0]))
    return normalize(value,roots[0]),execution


def report_rows(rows,constructor):
    """Construct passive finding or exception DTOs from native output."""
    errors=[r['parameters'] for r in rows if r['code']=='SF_ERROR']
    if errors:
        from pydantic import ValidationError
        compare(len(errors),1)
        details=json.loads(errors[0]['reconstruction_json'])
        for item in details:item['loc']=tuple(item['loc'])
        error=ValidationError.from_exception_data('ServerPlanFileFacts',details)
        error.args=tuple(json.loads(errors[0]['error_json'])['args'])
        raise error
    result=[]
    for row in sorted((r for r in rows if r['code']=='SF_REPORT'),key=lambda r:tuple(r['parameters'][k]
            for k in ['order0','order1','order2','order3','order4','bundle','ordinal'])):
        p=row['parameters'];result.append(constructor(linter_name='serverplan_grammar_surface',code=p['source_code'],
            path=Path(p['file']),line=json.loads(p['line_json']),message=p['source_message'],severity=p['severity']))
    return result
