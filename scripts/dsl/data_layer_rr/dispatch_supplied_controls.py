"""Require successful native runs to expose removed supplied-fact semantics."""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile
from migration import ROOT,PIN,compare,git,read_json,write_json,sha
from dispatch_reporter_port import execute_batch


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    a=p.parse_args();source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
    compare(baseline['sourceSha256'],sha((source/'scripts/lint_serverplan_dispatch_architecture.py').read_bytes()))
    corpus=[r['input'] for r in baseline['cases']]
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr'
    library=ROOT/'docs/rule-syntax/lib/data-layer/dispatch-supplied.rr';text=library.read_text()
    changes=[
        ('first-error','first-error',text.replace('(b2<b;b2=b,r2<r;b2=b,r2=r,f2<f)','(b2>b;b2=b,r2>r;b2=b,r2=r,f2>f)')),
        ('line-field','container-8',text.replace('dss_integer(c,b,r,"line",l,"",_)','dss_integer(c,b,r,"occurrence",l,"",_)')),
        ('late-boundary','late-boundary-before-invalid-values',text.replace('!sd_late(p),','')),
        ('duplicates','container-9',text.replace('DispatchSuppliedFact(@place("",0),c,b,r,0,','DispatchSuppliedFact(@place("",0),c,b,0,0,').replace('DispatchSuppliedReport(@place("",0),c,b,r,0,','DispatchSuppliedReport(@place("",0),c,b,0,0,')),
    ]
    from data_layer_audit.native_guard import codes
    original=codes({'serverplan_dispatch_architecture':module});fallbacks=[]
    def guard(frame,event,_arg):
        if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
            fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    rows=[]
    for name,target,changed in changes:
        if changed==text:raise AssertionError('Control did not change its requirement')
        content=policy.read_text().replace('#include "data-layer/dispatch-supplied.rr"',changed)
        if content==policy.read_text():raise AssertionError('Control library was not replaced')
        with tempfile.TemporaryDirectory(prefix='dispatch-supplied-control-',dir=ROOT/'.rerail') as temporary:
            control=Path(temporary)/'control.rr';control.write_text(content)
            sys.setprofile(guard)
            try:actual,execution=execute_batch(a.engine.absolute(),module,corpus,'snapshot',control)
            finally:sys.setprofile(None)
        compare(len(actual),len(corpus));index=next(i for i,c in enumerate(corpus) if c['id']==target)
        expected=baseline['cases'][index]['expected']
        if actual[index]==expected:raise AssertionError('Control was not detected: '+name)
        if name=='first-error':compare(actual[index]['findings']['error']['type'],'ValueError')
        if name=='line-field':compare(len(actual[index]['findings']['value']),len(expected['findings']['value']))
        if name=='late-boundary':compare(actual[index]['findings']['error']['type'],'TypeError')
        if name=='duplicates':compare(len(actual[index]['findings']['value']),len(expected['findings']['value'])-1)
        rows.append(dict(name=name,target=target,passed=True,expected=expected,actual=actual[index],execution=execution,mutatedPolicy=content,mutatedPolicySha256=sha(content.encode())))
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_reporter_port.py','dispatch_supplied_facts.py','python_value_transport.py','fact_transport.py','replay.py','migration.py']]]
    report=dict(format='rerail.dispatch-supplied-controls/v1',admitted=False,passed=not fallbacks,cases=rows,semanticFallbacks=fallbacks,
        engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files})
    write_json(a.out,report);print(dict(controls=len(rows),passed=report['passed']))


if __name__=='__main__':main()
