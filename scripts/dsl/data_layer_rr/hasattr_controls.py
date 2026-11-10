"""Prove source positives detect disabled async rule families and protocols."""
import argparse
import json
from pathlib import Path
import tempfile
from migration import ROOT,PIN,sha,write_json,compare
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/hasattr-async.rr';text=policy.read_text()
    cli=(ROOT/'docs/rule-syntax/lib/data-layer/text-cli.rr').read_text();empty='\n.decl async_disabled()\n'
    def disable(prefix):
        matches=[line for line in text.splitlines() if line.startswith(prefix) and ' :- ' in line]
        if not matches:raise AssertionError('Missing control producer: '+prefix)
        return '\n'.join(line.replace(' :- ',' :- async_disabled(), ',1) if line in matches else line for line in text.splitlines())+'\n'+empty
    def source(value):return [dict(path='input.py',source=value)]
    ordinary=dict(entryPoint='check_source')
    specs=[
        ('hasattr-family',source('hasattr(txn,"fetch_async")'),ordinary,disable('ppe_Asynchasattr('),(1,0)),
        ('getattr-family',source('getattr(txn,"fetch_async",None)'),ordinary,disable('ppe_Asyncgetattr('),(1,0)),
        ('allowed-line',source('hasattr(txn,"fetch_async"); update_async'),ordinary,disable('async_allowed('),(0,1)),
        ('excluded-path',[],dict(entryPoint='should_exclude',path='/repo/tests/a.py'),disable('async_excluded('),(1,0)),
        ('batch-selection',[],dict(entryPoint='_async_prepare',asyncPaths=['/repo/a.py','/repo/tests/a.py']),
            text.replace(', !async_excluded(p).','.'),(1,2)),
        ('native-facts',source('hasattr(txn,"fetch_async")'),dict(entryPoint='_async_facts'),disable('AsyncFact('),(2,1)),
        ('extractor-metadata',[],dict(entryPoint='_async_configuration'),text.replace('hasattr-async-v3','broken-version'),None),
    ]
    old=next(line for line in text.splitlines() if line.startswith('async_line('))
    changed=text.replace(old,'async_line(d,l,t) :- async_selected(d), split_line(d,l,t), !async_allowed(d,l).')
    specs.append(('comment-line',source('# hasattr(txn,"fetch_async")'),ordinary,changed,(0,1)))
    changed_cli='\n'.join(line.replace(' :- ',' :- async_disabled(), ',1) if line.startswith('TextCliOutput(') else line for line in cli.splitlines())+'\n'
    specs.append(('cli-rendering',[],dict(entryPoint='_cli_report',cliFile=['a.py'],cliLine=['2'],cliPattern=['hasattr'],cliMethod=['fetch_async']),
                  text.replace('#include "data-layer/text-cli.rr"',changed_cli)+empty,None))
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    report=dict(format='rerail.hasattr-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
        policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    with tempfile.TemporaryDirectory(prefix='hasattr-controls-',dir=ROOT/'.rerail') as temporary:
        for name,files,options,mutation,counts in specs:
            if mutation==text:raise AssertionError('Control changed nothing: '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            a=execute_native(engine,policy,files,options);b=execute_native(engine,path,files,options)
            if counts is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),counts)
            if a['diagnostics']==b['diagnostics']:raise AssertionError('Disabled behavior was not detected: '+name)
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();value=run(a.engine.absolute());write_json(a.out,value);print(dict(controls=len(value['cases']),passed=True))
