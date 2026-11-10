"""Compare complete native exceptions against the pinned source's public scanner."""
import json
from dto import canonical
from migration import ROOT

FIXTURE='native/src/code_rule_dsl/facts/vtab/syntax/python_syntax/python-exceptions.json'


def cases(_module):
    rows=[dict(id=f'exception-{i:03}',source=row['source'],filename='semantic_plan.py')
          for i,row in enumerate(json.loads((ROOT/FIXTURE).read_text()))]
    for filename in ['nested/semantic.py','dollar-$1.py','quote-"-\\.py','unicodé-😀.py']:
        rows.append(dict(id='filename-'+str(len(rows)),source='def broken(:\n',filename=filename))
    return rows


def exercise(module,case,temporary):
    def normalize(value):
        if isinstance(value,str):return value.replace(str(temporary),'$TMP')
        if isinstance(value,list):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    try:result=dict(value=canonical(module.scan_semantic_source(temporary/case['filename'],case['source'])))
    except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(result)


if __name__=='__main__':
    from contract_runner import main
    from composition_port import install
    main('plan_composition_architecture','rerail.composition-exception-contract/v1',cases,exercise,install,
         ['composition_exception_contract.py','composition_port.py','contract_runner.py','../../../'+FIXTURE])
