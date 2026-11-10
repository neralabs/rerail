"""Compare batched native Finding projections with pinned source observations."""
import argparse
import importlib
from pathlib import Path
import subprocess
import sys
from migration import ROOT,PIN,compare,git,read_json,write_json,sha


def cases():
    rows=[]
    def add(name,path='impl/a.py',root='/repo',expression='a  +\tb',**fields):
        row=dict(code='SPD001',path=path,line=1,scope='<module>',expression=expression,message='guidance',occurrence=1)
        row.update(fields);rows.append(dict(id=name,finding=row,root=root))
    add('default');add('empty',expression='',scope='',message='',code='')
    add('unicode',path='impl/é.py',expression='  α\u2000+\u0085β\n',scope='类.方法',message='消息')
    for char in ['\t','\n','\r','\v','\f','\x1c','\x1d','\x1e','\x1f','\u0085','\u00a0','\u1680','\u2000','\u2028','\u2029','\u202f','\u205f','\u3000','\u200b','\0']:
        add('whitespace-'+str(ord(char)),expression=char+'a'+char+char+'b'+char)
    for i,(path,root) in enumerate([('/repo/a.py','/repo'),('/repository/a.py','/repo'),('/repo','/repo'),
        ('/a.py','/'),('/','/'),('//a.py','//'),('//','//'),('//a.py','/'),('/a.py','//'),
        ('relative/a.py','.'),('/absolute.py','.'),('relative/a.py','relative'),('a.py',''),
        ('/repo/../a.py','/repo'),('/repo/a.py','/repo/..'),('/repo//./a.py','/repo/'),
        ('//repo/a.py','//repo'),('///repo/a.py','/repo'),('impl/../a.py','/repo')]):
        add('paths-'+str(i),path=path,root=root)
    for value in [0,-1,10**90]:add('numbers-'+str(value),line=value,occurrence=value)
    add('delimiters',path='a|b.py',expression='a|b',scope='s|x',message='a:\nb',code='CUSTOM',line=-42,occurrence=12)
    return rows


def payload(rows):
    names=['code','path','line','scope','expression','message','occurrence']
    result={ 'dispatchFinding'+name.title():[str(Path(r['finding'][name])) if name=='path' else str(r['finding'][name]) for r in rows] for name in names }
    result['dispatchFindingRoot']=[str(Path(r['root'])) for r in rows]
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','engine','baseline','out']:p.add_argument('--'+n,type=Path,required=n in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');a=p.parse_args()
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    corpus=cases();rows=[];fallbacks=[];native=None
    if a.command=='source':
        for c in corpus:
            finding=module.Finding(**dict(c['finding'],path=Path(c['finding']['path'])))
            rows.append(dict(input=c,expected=dict(inventory=finding.inventory_entry,rendered=finding.render(Path(c['root'])))))
    else:
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
        compare(baseline['sourceSha256'],sha(path.read_bytes()));compare([r['input'] for r in baseline['cases']],corpus)
        from data_layer_audit.native_guard import codes
        original=codes({'serverplan_dispatch_architecture':module})
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original and not original[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
        from replay import execute_native,execute_workspace
        execute=execute_native if a.mode=='snapshot' else execute_workspace
        sys.setprofile(guard)
        try:native=execute(a.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',[],dict(entryPoint='_finding_projection',**payload(corpus)))
        finally:sys.setprofile(None)
        groups={}
        for r in native['diagnostics']:
            compare(r['code'],'SD_FINDING');v=r['parameters'];i=v['ordinal']
            if i in groups:raise ValueError('Duplicate Finding projection')
            groups[i]=dict(inventory=v['inventory'],rendered=v['rendered'])
        for i,c in enumerate(corpus):
            expected=baseline['cases'][i]['expected'];actual=groups.get(i)
            rows.append(dict(input=c,expected=expected,actual=actual,passed=actual==expected))
        compare(set(groups)-set(range(len(corpus))),set())
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['replay.py','migration.py']]]
    report=dict(format='rerail.dispatch-finding-contract/v1',sourcePin=PIN,sourceSha256=sha(path.read_bytes()),oracle=a.command=='source',
        mode=a.mode,admitted=False,cases=rows,passed=all(r.get('passed',True) for r in rows) and not fallbacks,
        semanticFallbacks=fallbacks,harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files},native=native)
    if a.command=='native':report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
    write_json(a.out,report);print(dict(cases=len(rows),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
