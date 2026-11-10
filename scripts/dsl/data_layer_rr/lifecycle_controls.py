"""Disable lifecycle behaviors individually and retain complete native counterexamples."""
import argparse
import json
from pathlib import Path
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native

WITNESS='class BackendVersionedWriteTransaction:\n def commit(self): pass\n'
NESTED='class OuterVersionedWriteTransaction:\n def commit(self):\n  class InnerVersionedWriteTransaction:\n   def rollback(self): pass\n def commit_async(self): pass\n'


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/versioned-lifecycle-legacy-surfaces.rr'
    text=policy.read_text();library=(ROOT/'docs/rule-syntax/lib/data-layer/lifecycle-api.rr').read_text()
    def inline(changed):return text.replace('#include "data-layer/lifecycle-api.rr"',changed)
    def source(s):return [dict(path='input.py',source=s)]
    ordinary=dict(entryPoint='scan_file')
    scope=dict(entryPoint='scan_file',lifecycleBatch='true',lcRoot='/project',lcScopeRoot='/project',
               ownerDocs=['input.py'],ownerPaths=['/project/a.py'])
    def values(path='a.py'):
        return [dict(path='lifecycle-values.json',source=json.dumps(dict(values=[dict(path='/project/'+path,line=2,class_name='C',method_name='commit')])))]
    specs=[
        ('method',source(WITNESS),ordinary,text.replace('lifecycle_method("commit").',''),1,0),
        ('class-shape',source('class BackendWriteTransaction:\n def commit(self): pass\n'),ordinary,
         text+'\nlifecycle_class(c,@nfkc(raw)) :- kind(c,"class_definition"), def_name(c,raw).\n',0,1),
        ('direct-body',source('class BackendVersionedWriteTransaction:\n if True:\n  def commit(self): pass\n'),ordinary,
         text.replace('field(c, "body", b), body_stmt(b, st, _), defstmt(f, st)','within(f,c)'),0,1),
        ('supplied-trigger',source(WITNESS),dict(scope,lcSuppliedSource='pass\n'),text+'\nlc_supplied_ok().\n',0,1),
        ('root-scope',source(WITNESS),dict(scope,ownerPaths=['/outside/a.py']),text+'\nlc_relative(p,r,p) :- lc_rel_input(p,r).\n',0,1),
        ('test-scope',source(WITNESS),dict(scope,ownerPaths=['/project/tests/a.py']),inline(library.replace('!lc_test(i),','')),0,1),
        ('collector-order',source(NESTED),ordinary,text.replace('visit_index(c,ppe_class_order)','ppe_class_order=ppe_method_order'),None,None),
        ('renderer',values(),dict(entryPoint='_lc_cli_report',lcRoot='/project',checkMode='true'),
         inline(library.replace('base LazyVersionedWriteTransaction @legacy lifecycle surface','DISABLED')),None,None),
        ('fact-split',values('impl/a: b: c.py'),dict(entryPoint='_lc_fact',lcRoot='/project'),inline(library.replace('i < 2.','i < 1.')),None,None),
        ('discovery-duplicates',[],dict(entryPoint='_lc_files',candidatePaths=['a.py','a.py','dir.py','a.txt'],
             candidateKinds=['direct','direct','walk','direct'],candidateGroups=['0','1','2','3']),
         inline(library.replace('LifecycleFile(@place("",0),g,@replace_all(p,"/","\\u0000"),i,p)',
                                'LifecycleFile(@place("",0),0,@replace_all(p,"/","\\u0000"),0,p)')),3,2)]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    report=dict(format='rerail.lifecycle-controls/v1',sourcePin=PIN,oracle=False,passed=False,
        engineSha256=sha(engine.read_bytes()),policySha256=sha(policy.read_bytes()),harnessFiles=hashes,cases=[])
    folder=ROOT/'.rerail/lifecycle-finalec3';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,options,mutation,a_count,b_count in specs:
            if mutation==text:raise AssertionError('Control did not change behavior: '+name)
            changed=Path(temporary)/(name+'.rr');changed.write_text(mutation)
            a=execute_native(engine,policy,files,options);b=execute_native(engine,changed,files,options)
            if a_count is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),(a_count,b_count))
            elif name=='collector-order':
                def lines(r):return [x['line'] for x in sorted(r['diagnostics'],key=lambda x:(x['parameters']['collector_order0'],x['parameters']['collector_order1']))]
                compare(lines(a),[2,5,4]);compare(lines(b),[2,4,5])
            elif name=='renderer':
                out=lambda r:[x['parameters']['stdout'] for x in r['diagnostics'] if x['code']=='LC_OUTPUT']
                compare(len(out(a)),1);compare(out(b),[out(a)[0].replace('base LazyVersionedWriteTransaction @legacy lifecycle surface','DISABLED')])
            else:
                fact=lambda r:json.loads(next(x['parameters']['fact_json'] for x in r['diagnostics'] if x['code']=='LC_VALUE'))
                assert fact(a)['message'].startswith('c.py:2: C.commit ')
                assert fact(b)['message'].startswith('b: c.py:2: C.commit ')
            assert a['diagnostics']!=b['diagnostics']
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();value=run(a.engine.absolute());write_json(a.out,value);print(dict(controls=len(value['cases']),passed=True))


if __name__=='__main__':main()
