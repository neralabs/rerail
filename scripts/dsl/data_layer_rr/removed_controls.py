"""Disable independent RemovedCompat behaviors and verify observable failures."""
import argparse
from pathlib import Path
import tempfile
from migration import ROOT,PIN,sha,write_json,compare
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/removed-compat-surfaces.rr'; text=policy.read_text()
    def source(s): return [dict(path='input.py',source=s)]
    ordinary=dict(entryPoint='scan_file')
    disabled='\n.decl rcs_disabled()\n'
    specs=[
        ('name-family',source('TimerClaim\n'),ordinary,
         text.replace('removed_symbol(n, @nfkc(raw)) :- ast_name(n, raw),','removed_symbol(n, @nfkc(raw)) :- rcs_disabled(), ast_name(n, raw),')+disabled,1,0),
        ('attribute-family',source('obj.TimerClaim\n'),ordinary,
         text.replace('removed_symbol(n, @nfkc(raw)) :- kind(n, "attribute"),','removed_symbol(n, @nfkc(raw)) :- rcs_disabled(), kind(n, "attribute"),')+disabled,1,0),
        ('definition-family',source('def TimerClaim(): pass\n'),ordinary,
         text.replace('removed_symbol(n, @nfkc(raw)) :- def_name(n, raw),','removed_symbol(n, @nfkc(raw)) :- rcs_disabled(), def_name(n, raw),')+disabled,1,0),
        ('api-trigger-bypass',source('ＴimerClaim\n'),ordinary,
         text.replace('option("entryPoint", "scan_file").','option("entryPoint", "scan_file"), pp_triggered(d).'),1,0),
        ('same-line-dedup',source('TimerClaim; TimerClaim\n'),ordinary,
         text.replace('removed_first(d, l, x, first) :- removed_hit(d, l, x, _), first = min i : { removed_hit(d, l, x, i) }.',
                      'removed_first(d, l, x, i) :- removed_hit(d, l, x, i).'),1,2),
        ('file-suffix',[],dict(entryPoint='_rcs_files',candidatePaths=['/project/a.txt','/project/a.py','/project/a.py']),
         text+'\nrcs_file(p,@replace_all(p,"/","\\u0000")) :- option("entryPoint","_rcs_files"), option_item("candidatePaths",_,p).\n',1,2),
        ('cli-mode',[],dict(entryPoint='_rcs_cli_report',cliPath=['a.py'],cliLine=['2'],cliLineJson=['2'],
             cliCode=['RCS001'],cliMessage=['message'],checkMode='true',jsonMode='false',jsonDetailsMode='false'),
         text+'\nrcs_json() :- rcs_cli_count(_).\n',None,None)]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    report=dict(format='rerail.removed-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
                policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    folder=ROOT/'.rerail/removed-finalec3'; folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,options,mutation,a_count,b_count in specs:
            if mutation==text: raise AssertionError('Control did not mutate behavior')
            changed=Path(temporary)/(name+'.rr'); changed.write_text(mutation)
            original=execute_native(engine,policy,files,options); altered=execute_native(engine,changed,files,options)
            a,b=original['diagnostics'],altered['diagnostics']
            if a_count is not None: compare((len(a),len(b)),(a_count,b_count))
            else:
                def stdout(rows): return ''.join(r['parameters']['stdout'] for r in sorted(
                    (r for r in rows if r['code']=='RCS_OUTPUT'),key=lambda r:(r['parameters']['phase'],r['parameters']['ordinal'])))
                compare(stdout(a),'a.py:2: RCS001 message\nremoved_compat_surfaces: 1 violation(s)\n')
                assert stdout(b).startswith('[\n') and stdout(b).endswith('\n]\n')
            assert a!=b
            report['cases'].append(dict(name=name,passed=True,original=original,disabled=altered,
                                       mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items(): compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True; return report


def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args(); value=run(a.engine.absolute()); write_json(a.out,value); print(dict(controls=len(value['cases']),passed=True))


if __name__=='__main__': main()
