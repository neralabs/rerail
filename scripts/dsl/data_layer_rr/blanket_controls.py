"""Disable grounded bypass producers and check complete native counterexamples."""
import argparse
import json
from pathlib import Path
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native,execute_workspace


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/no-blanket-grammar-bypass.rr';text=policy.read_text()
    api=(ROOT/'docs/rule-syntax/lib/data-layer/blanket-bypass-api.rr').read_text()
    cli=(ROOT/'docs/rule-syntax/lib/data-layer/blanket-bypass-cli.rr').read_text()
    sql='impl/common/l2200_sql_dialect/sql_connection.py';empty='\n.decl bypass_disabled()\n'
    def files(source,path='api/a.py'):return [dict(path=path,source=source)]
    def disable(body,prefix):
        lines=body.splitlines();selected=[line for line in lines if line.startswith(prefix) and ' :- ' in line]
        if not selected:raise AssertionError('Missing native producer: '+prefix)
        return '\n'.join(line.replace(' :- ',' :- bypass_disabled(), ',1) if line in selected else line for line in lines)+'\n'
    def inline(body):return text.replace('#include "data-layer/blanket-bypass-api.rr"',body)+empty
    ordinary=dict(entryPoint='check_no_bypass_references')
    specs=[
        ('constant',files('_ALLOW_DIRECT_EXECUTION'),ordinary,disable(text,'ppe_BypassConstant(')+empty,(1,0)),
        ('argument',files('allow_direct_execution'),ordinary,disable(text,'ppe_BypassArgument(')+empty,(1,0)),
        ('method',files('def execute_grammar(): pass\n',sql),dict(entryPoint='check_no_public_grammar_methods'),disable(text,'ppe_BypassPublic(')+empty,(1,0)),
        ('allowed-file',files('_ALLOW_DIRECT_EXECUTION','scripts/lint_impl_direct_execution.py'),ordinary,text.replace('!bypass_allowed(p),',''),(0,1)),
        ('nfkc-method',files('# execute_grammar\ndef ｅxecute_grammar(): pass\n',sql),dict(entryPoint='check_no_public_grammar_methods'),text.replace('n=@nfkc(raw)','n=raw'),(1,0)),
        ('read-allowlist',[],dict(entryPoint='_bypass_prepare_refs',bypassCandidates=['api/a.py','scripts/lint_impl_direct_execution.py']),inline(api.replace(', !bypass_allowed(p).','.')),(1,2)),
        ('fact-suffix',[],dict(entryPoint='_bypass_prepare_facts',bypassCandidates=['api/a.py','.py'],bypassIsFile=['true','true']),inline(api.replace('))>3', '))>=3')),(1,2)),
    ]
    old=next(line for line in text.splitlines() if line.startswith('bypass_scan('))
    changed=text.replace(old,'bypass_scan(d,p,l,t) :- bypass_doc(d,p), bypass_reference_doc(d), bypass_selected(d), split_line(d,l,t), !bypass_allowed(p).')
    specs.append(('comment',files('# _ALLOW_DIRECT_EXECUTION'),ordinary,changed,(0,1)))
    split_files=[dict(path='supplied.py',source='_ALLOW_DIRECT_EXECUTION'),dict(path='disk.py',source='def execute_grammar(): pass\n')]
    options=dict(entryPoint='scan_file_content',bypassDocs=['supplied.py','disk.py'],bypassPaths=[sql,sql],bypassRoles=['source','disk'])
    specs.append(('supplied-gate',split_files,options,inline(api.replace(', bypass_supplied_method(p), option("entryPoint","scan_file_content")',', option("entryPoint","scan_file_content")')),(2,4)))
    options_facts=dict(entryPoint='_bypass_facts',bypassDocs=['disk.py'],bypassPaths=[sql],bypassRoles=['disk'])
    specs.append(('collector-parse',files('_ALLOW_DIRECT_EXECUTION\ndef broken(:\n','disk.py'),options_facts,
        inline(api.replace('option("entryPoint","_bypass_facts"), !syntax_error(d,_,_).','option("entryPoint","_bypass_facts").')),(0,2)))
    changed='\n'.join(line.replace('i,a,b,c,','i,-1,b,c,',1) if line.startswith('BypassFact(') and 'public method ' in line else line for line in api.splitlines())+'\n'
    specs.append(('bypass-collector-order',files('def execute_grammar(): pass\n_ALLOW_DIRECT_EXECUTION\n','disk.py'),options_facts,inline(changed),None))
    specs.append(('bypass-extractor',[],dict(entryPoint='_configuration'),text.replace('no-blanket-grammar-bypass-v2','broken-version'),None))
    options_cli=dict(entryPoint='_cli_report',bypassRows=['  a.py:1: bypass'],methodRows=[],checkMode='0')
    changed=cli.replace('option("checkMode","0")','option("checkMode","SWAP")').replace('option("checkMode","1")','option("checkMode","0")').replace('option("checkMode","SWAP")','option("checkMode","1")')
    specs.append(('bypass-check-flag',[],options_cli,text.replace('#include "data-layer/blanket-bypass-cli.rr"',changed),None))
    specs.append(('bypass-renderer',[],options_cli,text.replace('#include "data-layer/blanket-bypass-cli.rr"',disable(cli,'BypassOutput('))+empty,None))
    specs.append(('alias-scope',[dict(path=sql,source='def execute_grammar(): pass\n'),
        dict(path='impl/common/l2200_sql_dialect/alias.py',symlink='sql_connection.py')],{},text.replace(', ppe_path=p',''),(1,2)))
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['replay.py','migration.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    report=dict(format='rerail.blanket-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
        policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    with tempfile.TemporaryDirectory(prefix='blanket-controls-',dir=ROOT/'.rerail') as temporary:
        for name,documents,options,mutation,counts in specs:
            if mutation==text:raise AssertionError('No mutation for '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            execute=execute_workspace if name=='alias-scope' else execute_native
            a=execute(engine,policy,documents,options);b=execute(engine,path,documents,options)
            if counts is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),counts)
            if a['diagnostics']==b['diagnostics']:raise AssertionError('Undetected disabled behavior: '+name)
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();value=run(a.engine.absolute());write_json(a.out,value);print(dict(controls=len(value['cases']),passed=True))
