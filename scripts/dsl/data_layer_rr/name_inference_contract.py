"""Record name-inference scanners, default iterables, facts and complete CLI streams."""
import contextlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='serverplan_name_inference'
WITNESS='column == "count"\n'


def cases(module):
    rows=[]
    for value in module._CONTROL_COLUMNS:
        for i,shape in enumerate(['column == {v}','{v} != column','set(row) == {{{v}}}','row.keys() != {{{v}}}']):
            rows.append(dict(kind='source',id=f'control-{value}-{i}',source=shape.format(v=repr(value))))
    for suffix in module._REFERENCE_SUFFIXES:
        for method in module._REFERENCE_TEXT_METHODS:
            rows.append(dict(kind='source',id=f'parse-{suffix}-{method}',source=f'source_ref.{method}({suffix!r})'))
        for method in ['get','pop','setdefault']:
            rows.append(dict(kind='source',id=f'synthesis-{suffix}-{method}',source=f'ctx.refs.{method}(f"{{source_ref}}{suffix}")'))
    for method in module._REFERENCE_TEXT_METHODS:
        for filename in ['sql_column_type_hints.py','runtime.py']:
            rows.append(dict(kind='source',id=f'type-{method}-{filename}',path=filename,source=f'column.{method}("_id")'))
    for label,text in [('clean','pass'),('comment','# '+WITNESS),('indented-comment','\u2003 # '+WITNESS),
        ('inline-comment','pass # '+WITNESS),('upper-control','column == "COUNT"'),
        ('upper-reference','SOURCE_REF.ENDSWITH(".RAW")'),('unicode-fold','ref.ſplit(".raw")'),
        ('missing-literal','column == value'),('missing-shape','value == "count"'),
        ('raw-string','text = \'column == "count"\''),('invalid-python','if (: '+WITNESS),
        ('all-codes','column == "count"; ref.endswith(".raw"); refs.get(f"{ref}.raw"); column.split("_")'),
        ('duplicates',WITNESS+WITNESS),('one-per-line',WITNESS.strip()+'; '+WITNESS.strip())]:
        rows.append(dict(kind='source',id=label,path='sql_column_type_hints.py',source=text))
    for sep in ['\n','\r\n','\r','\v','\f','\x1c','\x85','\u2028','\u2029']:
        rows.append(dict(kind='source',id='separator-'+repr(sep),source='pass'+sep+WITNESS))
    for path in ['a.txt','.py','tests/a.py','__pycache__/a.py','nested/sql_column_type_hints.py','SQL_COLUMN_TYPE_HINTS.py']:
        rows.append(dict(kind='source',id='path-'+path,path=path,source='column.startswith("_")\n'+WITNESS))
    for text in [WITNESS,'ref.endswith(".raw")','refs.get(f"{ref}.raw")','column.split("_")',WITNESS+WITNESS,'pass']:
        rows.append(dict(kind='facts',id='facts-'+str(len(rows)),source=text,path='sql_column_type_hints.py'))
    for group in ['none','empty-list','empty-tuple','empty-generator','one','duplicates','basename-collision',
                  'lexical-lines','non-python','missing','invalid-utf8','explicit-test','custom-default','overlap-roots']:
        rows.append(dict(kind='violations',id='violations-'+group,group=group))
    for group in ['ordinary','overlap','missing','directory-py']:
        rows.append(dict(kind='paths',id='paths-'+group,group=group))
    for group in ['clean','one','duplicates','ordering']:
        for args in [[],['--check']]:rows.append(dict(kind='cli',id='cli-'+group+repr(args),group=group,args=args))
    for args in [['--help'],['--bad']]:rows.append(dict(kind='cli',id='cli-special-'+repr(args),group='clean',args=args))
    rows.append(dict(kind='cli-run',id='cli-native-scan',args=['--check']))
    rows.append(dict(kind='configuration',id='configuration'))
    return rows


def exercise(module,case,temporary):
    root=temporary/'project';root.mkdir();calls=[]
    def normalize(value):
        if isinstance(value,Path):return normalize(str(value))
        if isinstance(value,str):return value.replace(str(temporary),'$TMP')
        if isinstance(value,(tuple,list)):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'_SCAN_ROOTS',(root/'scan',)))
        kind=case['kind']
        if kind in {'source','facts'}:
            path=root/case.get('path','a.py')
            action=lambda:module._scan_source(path,case['source']) if kind=='source' else module._extract_name_inference_facts(path,case['source'])
        elif kind in {'paths','violations','cli-run'}:
            for p in ['scan/a.py','scan/tests/a.py','scan/__pycache__/a.py','scan/test/a.py','scan/z.py','other/a.py','scan/a.txt']:
                path=root/p;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(WITNESS)
            group=case.get('group','none')
            if group in {'overlap','overlap-roots'}:stack.enter_context(patch.object(module,'_SCAN_ROOTS',(root/'scan',root/'scan')))
            if kind=='paths':
                if group=='missing':stack.enter_context(patch.object(module,'_SCAN_ROOTS',(root/'missing',)))
                if group=='directory-py':(root/'scan/directory.py').mkdir()
                action=module._default_paths
            elif kind=='cli-run':
                stack.enter_context(patch.object(sys,'argv',['lint_serverplan_name_inference.py',*case['args']]))
                action=module.cli_main
            else:
                if group=='lexical-lines':(root/'scan/a.py').write_text(WITNESS*12)
                if group=='invalid-utf8':(root/'bad.py').write_bytes(b'\xff')
                if group=='custom-default':
                    def defaults():calls.append(dict(name='defaults'));return (root/'other/a.py',)
                    stack.enter_context(patch.object(module,'_default_paths',defaults))
                paths={'none':None,'empty-list':[],'empty-tuple':(),'empty-generator':iter(()),
                    'one':(root/'scan/a.py',),'duplicates':(root/'scan/a.py',root/'scan/a.py'),
                    'basename-collision':(root/'scan/a.py',root/'other/a.py'),
                    'lexical-lines':(root/'scan/a.py',),'non-python':(root/'scan/a.txt',),
                    'missing':(root/'missing.py',),'invalid-utf8':(root/'bad.py',),
                    'explicit-test':(root/'scan/tests/a.py',),'custom-default':None,'overlap-roots':None}[group]
                action=lambda:module.violations(paths)
        elif kind=='cli':
            def findings():
                calls.append(dict(name='violations'))
                return dict(clean=[],one=['a.py:2: SPNI001 message'],duplicates=['a.py:2: SPNI001 message']*2,
                    ordering=['z.py:12: SPNI003 message','a.py:2: SPNI001 message'])[case['group']]
            stack.enter_context(patch.object(module,'violations',findings))
            stack.enter_context(patch.object(sys,'argv',['lint_serverplan_name_inference.py',*case['args']]))
            action=module.cli_main
        else:
            action=lambda:dict(columns=module._CONTROL_COLUMNS,suffixes=module._REFERENCE_SUFFIXES,
                methods=module._REFERENCE_TEXT_METHODS,directories=module._SCAN_DIRECTORIES,
                patterns={name:dict(pattern=getattr(module,name).pattern,flags=getattr(module,name).flags) for name in
                    ['_COLUMN_SHAPE_RE','_CONTROL_LITERAL_RE','_REFERENCE_PARSE_RE','_REFERENCE_SYNTHESIS_RE','_COLUMN_TYPE_RE']},
                extractor=canonical(module.NAME_INFERENCE_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=action())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls))


if __name__=='__main__':
    from contract_runner import main
    from name_inference_port import install
    main(CHECK,'rerail.name-inference-contract/v1',cases,exercise,install,['name_inference_contract.py','name_inference_port.py','contract_runner.py'])
