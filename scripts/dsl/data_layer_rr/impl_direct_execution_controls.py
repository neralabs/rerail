"""Require successful native scanner executions to reject disabled direct-execution semantics."""
import argparse
from pathlib import Path
from unittest.mock import patch

import impl_direct_execution_source_contract as contract
from migration import PIN,compare,read_json,sha,write_json


def mutations(text):
    edits=[('text-priority','pattern-priority','i = min j : { ide_text_event(d,p,l,j) }','i = max j : { ide_text_event(d,p,l,j) }'),
           ('ast-pass','spaced-_fetchone','ide_ast_choice(d,p,l,i,v) :- ide_ast_event','ide_ast_choice(d,p,l,i,v) :- 0 = 1, ide_ast_event'),
           ('masking','string','!ide_masked(m),','0 = 0,'),
           ('allowlist','allowed-connection','!ide_allowed(p), ide_pattern','0 = 0, ide_pattern'),
           ('fts','fts-direct','ide_fts(p), ide_pattern(i,pattern,attribute), i < 2','ide_fts(p), ide_pattern(i,pattern,attribute), i < 0'),
           ('source-gate','source-text-only-gate','ide_entry_allowed(p), ide_argument_gate(),','ide_entry_allowed(p),'),
           ('ast-order','ast-order','v = min w : { ide_ast_event(d,p,l,_,w) }','v = max w : { ide_ast_event(d,p,l,_,w) }'),
           ('text-before-ast','text-before-earlier-ast','ide_event(d,p,0,l,l,pattern)','ide_event(d,p,2,l,l,pattern)')]
    for name,case,old,new in edits:
        if old not in text:raise ValueError('Missing mutation target: '+name)
        yield name,case,text.replace(old,new)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out','work']:p.add_argument('--'+n,type=Path,required=True)
    a=p.parse_args();source=read_json(a.baseline);assert source['passed'] and source['sourcePin']==PIN
    compare(contract.CASES,[c['input'] for c in source['cases']]);original=contract.RULE;text=original.read_text();a.work.mkdir(parents=True,exist_ok=True)
    result=dict(format='rerail.impl-direct-controls/v1',sourcePin=PIN,passed=False,engineSha256=sha(a.engine.read_bytes()),ruleSha256=sha(text.encode()),cases=[])
    for name,label,changed in mutations(text):
        case=next(c for c in source['cases'] if c['input']['id']==label);path=(a.work/(name+'.rr')).absolute();path.write_text(changed)
        with patch.object(contract,'RULE',path):actual,native=contract.native_case(a.engine.absolute(),case['input'])
        if actual==case['expected']:raise ValueError('Incorrect policy passed: '+name)
        assert any(p['sha256']==sha(changed.encode()) for p in native['programInputs'])
        result['cases'].append(dict(id=name,input=case['input'],expected=case['expected'],actual=actual,native=native,mutant=changed));write_json(a.out,result)
    assert original.read_text()==text;result['passed']=True;write_json(a.out,result);print(dict(controls=len(result['cases']),passed=True))


if __name__=='__main__':main()
