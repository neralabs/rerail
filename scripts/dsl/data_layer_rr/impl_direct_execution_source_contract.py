"""Compare full direct-execution scanner values, including text/AST precedence and caller text gates."""
import argparse
from dataclasses import asdict
import importlib
from pathlib import Path
import subprocess
import sys
import tempfile

from migration import ROOT,PIN,compare,git,read_json,sha,write_json
from replay import execute_native

RULE=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/impl-direct-execution.rr'

def case(name,source,path='impl/custom/module.py',entry='scan_file',**extra):return dict(id=name,source=source,path=path,entry=entry,**extra)

ATTRS=['_execute_direct_internal','_execute_direct_internal_async','_fetchone','_fetchall','fetchone','fetchall','execute_grammar_fetchone','execute_grammar_fetchall']
CASES=[case('empty',''),case('clean','pass\n'),
    *[case('attribute-'+a,'obj.'+a+'()\n') for a in ATTRS],
    *[case('spaced-'+a,'obj.'+a+' ()\n') for a in ATTRS],
    case('pattern-priority','obj.fetchall(); obj._fetchone()\n'),
    case('ast-order','obj.fetchall (); obj._fetchone ()\n'),
    case('text-before-earlier-ast','obj.fetchall ()\nobj._fetchone()\n'),
    case('text-wins-same-line','obj.fetchall (); obj._fetchone()\n'),
    case('multiline-receiver','(\n obj\n).fetchone()\n'),
    case('multiline-attribute','obj.\\\n fetchone ()\n'),
    case('string','"obj._fetchone()"\n'),case('bytes','b"obj._fetchone()"\n'),
    case('raw-string','r"obj._fetchone()"\n'),case('triple-string','"""first\nobj._fetchone()\nlast"""\n'),
    case('comment','# obj._fetchone()\n'),case('inline-comment','pass # obj._fetchone()\n'),
    case('after-string','"obj._fetchall()"; obj._fetchone()\n'),
    case('unicode-before-comment','label="é😀" # obj._fetchone()\n'),
    case('unicode-before-code','label="é😀"; obj._fetchone()\n'),
    case('fstring-literal','f"obj._fetchone()"\n'),case('fstring-expression','f"{obj._fetchone()}"\n'),
    case('fstring-format','f"{obj:{obj._fetchone()}}"\n'),
    case('fstring-escaped-braces','f"{{obj._fetchone()}}"\n'),
    case('fstring-inner-string','f"{\'obj._fetchone()\'}"\n'),
    case('nested-fstrings','f"outer {f\'inner {obj._fetchone()}\'}"\n'),
    case('unicode-name','obj._ｆetchone()\n'),
    case('untriggered-unicode-name','obj.ｆetchone()\n'),
    case('syntax-error','def :\nobj._fetchone()\n'),
    case('invalid-literal','"unterminated\nobj._fetchone()\n'),
    case('allowed-connection','obj._fetchone()\n',path='impl/sqlite/connection.py'),
    case('fts-direct','obj._fetchone(); obj._execute_direct_internal()\n',path='impl/sqlite/fts/manager.py'),
    case('fts-spaced','obj._execute_direct_internal ()\n',path='impl/sqlite/fts/manager.py'),
    case('fts-only-outside','obj._execute_direct_internal()\n',entry='scan_fts_manager_direct_execution'),
    case('fts-only-inside','obj._execute_direct_internal()\n',path='impl/sqlite/fts/manager.py',entry='scan_fts_manager_direct_execution'),
    case('source-text-only-gate','obj._fetchone()\n',entry='scan_source',callSource='pass'),
    case('source-reads-disk','obj.fetchall()\n',entry='scan_source',callSource='obj._fetchone()'),
    case('fts-source-ignores-buffer','obj._execute_direct_internal()\n',path='impl/sqlite/fts/manager.py',entry='scan_fts_manager_source',callSource='pass'),
    case('form-feed','\fobj._fetchone()\n'),case('unicode-separator-in-string','"line\u2028text"; obj._fetchone()\n'),
    case('exact-trim','\tobj._fetchone()   \n'),case('long-line','obj._fetchone('+repr('x'*400)+')\n'),
    case('logical-separator-inside-string','"prefix\u2028obj._fetchone()"\n'),
    case('logical-separator-inside-comment','# prefix\u2028obj._fetchone()\n')]


def source_case(module,case):
    with tempfile.TemporaryDirectory(prefix='impl-direct-source-') as temporary:
        root=Path(temporary);path=root/case['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(case['source'])
        func=getattr(module,case['entry'])
        values=func(path,case.get('callSource',case['source'])) if case['entry'].endswith('_source') or case['entry']=='scan_source' else func(path)
        result=[]
        for value in values:
            row=asdict(value);row['file']=str(row['file']).replace(str(root),'$ROOT');result.append(row)
        return result


def native_case(engine,case):
    options=dict(entryPoint=case['entry'],argumentPath='$ROOT/'+case['path'],projectRoot='__outside__')
    if 'callSource' in case:options['callSource']=case['callSource']
    result=execute_native(engine,RULE,[dict(path='input.py',source=case['source'])],options)
    if any(r['code']!='IDE_ROW' for r in result['diagnostics']):raise ValueError('Unexpected scanner result')
    values=[{k:r['parameters'][k] for k in ['file','line_num','line','pattern']} for r in sorted(result['diagnostics'],key=lambda r:tuple(r['parameters'][k] for k in ['phase','ordinal']))]
    return values,result


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for n in ['source','baseline','engine','out']:p.add_argument('--'+n,type=Path)
    a=p.parse_args();report=dict(format='rerail.impl-direct-source-contract/v1',sourcePin=PIN,fullMigrationAcceptance=False,passed=False,cases=[])
    if a.command=='source':
        source=a.source.absolute();assert git(source,'rev-parse','HEAD')==PIN
        name='scripts/lint_impl_direct_execution.py';compare((source/name).read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+name]))
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_impl_direct_execution')
        for c in CASES:report['cases'].append(dict(input=c,expected=source_case(module,c)))
    else:
        if any(n.endswith('lint_impl_direct_execution') for n in sys.modules):raise ValueError('Native scanner tests must not import source detectors')
        baseline=read_json(a.baseline);assert baseline['passed'];compare(CASES,[c['input'] for c in baseline['cases']])
        report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),ruleSha256=sha(RULE.read_bytes()))
        for c in baseline['cases']:
            actual,native=native_case(a.engine.absolute(),c['input']);report['cases'].append(dict(c,actual=actual,native=native,passed=actual==c['expected']));write_json(a.out,report)
    report['passed']=all(c.get('passed',True) for c in report['cases']);write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
