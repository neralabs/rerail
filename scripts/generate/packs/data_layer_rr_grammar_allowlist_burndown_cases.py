"""Exercise native literal extraction, source quirks, CLI ownership, and ordered fact reporting."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];FIXTURES=ROOT/'gate/data-layer/fixtures/host/data-layer-port';CHECK='grammar_allowlist_burndown'
KEYS=['MAX_RENDER_STATIC_ALLOWLIST','MAX_RENDER_LEGACY_ALLOWLIST','MAX_ESCAPE_HATCH_ALLOWLIST','MAX_DATA_VALUE_PARAMS','MAX_OPERATOR_PARAMS']
VARS=['ALLOWLISTED_STATIC_CLASSES','LEGACY_ALLOWLISTED_CLASSES','ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS']
OWNERS=['lint_grammar_render_methods.py','lint_grammar_render_methods.py',*['lint_sql_grammar_escape_hatches.py']*3]


def main():
    api=[];reporters=[]
    def add(name,entry,files,arguments):api.append(dict(id=name,check=CHECK,entry=entry,files=files,arguments=arguments))
    expressions={
        'set':"{'a', 'b'}",'empty-string':"{''}",'unicode':"{'λ', 'é', '𐀀'}",'deduplicate':"{'a', 'a'}",'mixed':"{'a', 1, None, True, 1j}",
        'bytes':"{b'a'}",'formatted':"{f'a'}",'fstring-expr':"{f'{value}'}",'concatenated':"{'a' 'b'}",'escaped':"{'a\\n', '\\u03bb'}",'set-starred':"{*{'a'}, 'b'}",'comprehension':"{value for value in ['a']}",
        'union':"{'a'} | {'b'}",'union-nested':"({'a'} | {'b'}) | ({'c'} | {'d'})",'union-nonset':"other | {'a'}",'intersection':"{'a'} & {'b'}",'subtraction':"{'a'} - {'b'}",'tuple':"('a',)",'list':"['a']",'dict':"{'a': 'b'}",'name':'ALIAS',
        'frozen':"frozenset({'a'})",'other-call':"anything({'a'})",'attribute-call':"pkg.make({'a'})",'empty-call':'frozenset()',
        'call-union':"frozenset({'a'} | {'b'})",'call-list':"frozenset(['a'])",'call-tuple':"frozenset(('a',))",'call-second':"frozenset(1, {'a'})",'call-first':"frozenset({'a'}, 1)",
        'keyword-ignored':"frozenset(value={'a'})",'kw-before-pos':"frozenset(value={'ignored'}, *[{'a'}])",'starred-first':"frozenset(*{'a'})",'nested-call':"frozenset(frozenset({'a'}))",'union-call-arm':"frozenset({'a'} | frozenset({'b'}))",
        'parenthesized':"((frozenset(({'a'}))))",'union-floats':"1.5 | {'a'}",'set-parenthesized-element':"{('a')}",
    }
    for name,expr in expressions.items():
        for entry in ['_extract_set_values','_extract_frozenset_values']:
            add('helper-'+entry+'-'+name,entry,[],dict(node=dict(pythonAst=expr,mode='eval')))
        add('file-'+name,'_extract_frozenset_from_ast',[dict(path='case.py',source='FOO = '+expr+'\n')],dict(file_path=dict(path='case.py'),variable_name='FOO'))
    files={
        'first-nonempty':"FOO = frozenset()\nFOO = {'a'}\nFOO = {'b'}\n",'first-before-empty':"FOO = {'a'}\nFOO = frozenset()\n",'nested-dfs':"def first():\n    FOO = {'nested'}\nFOO = {'module'}\n",'class':'class C:\n    FOO = {"a"}\n',
        'chain':"FOO = BAR = {'a'}\n",'repeated-target':"FOO = FOO = {'a'}\n",'tuple-ignored':"FOO, BAR = {'a'}, {'b'}\n",'annotated':"FOO: frozenset[str] = {'a'}\n",'annotation-only':'FOO: frozenset[str]\n','attribute-ignored':"obj.FOO = {'a'}\n",'augmented-ignored':"FOO |= {'a'}\n",'walrus-ignored':"(FOO := {'a'})\n",'unicode-name':"ＦＯＯ = {'a'}\n",'empty':'','comments':'# FOO = {"fake"}\n','parse-error':'def broken(:\n',
    }
    for name,text in files.items():
        docs=[dict(path='case.py',source=text)]
        add('assignment-'+name,'_extract_frozenset_from_ast',docs,dict(file_path=dict(path='case.py'),variable_name='FOO'))
        add('facts-'+name,'_allowlist_facts_for_files',docs,dict(root=dict(path='.'),files={'tuple':[dict(path='case.py')]}))
    for path in ['tests/case.py','test/case.py','.venv/case.py','tmp/case.py','case.txt','case.PY']:
        add('explicit-'+path,'_extract_frozenset_from_ast',[dict(path=path,source="FOO = {'a'}\n")],dict(file_path=dict(path=path),variable_name='FOO'))
    add('source-ignored','_extract_frozenset_from_source',[dict(path='case.py',source="FOO = {'from-file'}\n")],dict(source="FOO = {'from-argument'}",variable_name='FOO',file_path=dict(path='case.py')))
    add('source-no-path','_extract_frozenset_from_source',[],dict(source="FOO = {'ignored'}",variable_name='FOO'))
    add('specs','_allowlist_specs',[],{})
    empty=[dict(path='scripts/lint_grammar_render_methods.py',source=''),dict(path='scripts/lint_sql_grammar_escape_hatches.py',source='')]
    def cli(name,docs,args=None,maxima=None):add('cli-'+name,'cli',docs,dict(root=dict(path='.'),args=args or [],maxima=maxima or {}))
    cli('empty',empty);cli('empty-verbose',empty,['--verbose']);cli('empty-check',empty,['--check'])
    cli('negative-maxima',empty,maxima={key:-1 for key in KEYS});cli('positive-maxima',empty,maxima={key:3 for key in KEYS})
    for i,(name,owner) in enumerate(zip(VARS,OWNERS)):
        docs=[dict(f,source=(name+" = frozenset({'b', 'a', 'a'})\n") if Path(f['path']).name==owner else '') for f in empty]
        cli(str(i)+'-grown',docs,['--check']);cli(str(i)+'-verbose',docs,['--verbose']);cli(str(i)+'-at-limit',docs,maxima={KEYS[i]:2});cli(str(i)+'-remaining',docs,maxima={KEYS[i]:3})
        wrong=[dict(f,source=(name+" = {'misplaced'}\n") if Path(f['path']).name!=owner else '') for f in empty];cli(str(i)+'-wrong-owner',wrong)
    cli('union-over-assignments',[dict(empty[0],source="ALLOWLISTED_STATIC_CLASSES = {'a'}\nALLOWLISTED_STATIC_CLASSES = {'b'}\n"),empty[1]],['--verbose'])
    cli('empty-does-not-clear',[dict(empty[0],source="ALLOWLISTED_STATIC_CLASSES = {'a'}\nALLOWLISTED_STATIC_CLASSES = frozenset()\n"),empty[1]],['--verbose'])
    cli('missing-render',[empty[1]]);cli('missing-escape',[empty[0]])
    cli('parse-error',[dict(empty[0],source='def broken(:\n'),empty[1]])
    cli('unicode-output',[dict(empty[0],source="ALLOWLISTED_STATIC_CLASSES = {'λ', 'é', 'A', '𐀀', ''}\n"),empty[1]],['--verbose'])
    for i,name in enumerate(VARS):
        for suffix,text in [('grown',name+" = {'b', 'a'}\n"),('last',name+" = {'a', 'b'}\n"+name+" = {'c'}\n"),('empty-retains',name+" = {'a'}\n"+name+" = frozenset()\n")]:
            reporters.append(dict(id='report-'+str(i)+'-'+suffix,check=CHECK,files=[dict(path='scripts/arbitrary.py',source=text)]))
    for path in ['scripts/a.py','scripts/tests/a.py','scripts/test/a.py','scripts/.venv/a.py','scripts/tmp/a.py','impl/a.py']:
        reporters.append(dict(id='scope-'+path,check=CHECK,files=[dict(path=path,source="ALLOWLISTED_CLASSES = {'a'}\n")]))
    reporters.append(dict(id='all-groups',check=CHECK,files=[dict(path='scripts/a.py',source=''.join(name+" = {'a'}\n" for name in reversed(VARS)))]))
    reporters.append(dict(id='duplicate-target',check=CHECK,files=[dict(path='scripts/a.py',source="ALLOWLISTED_CLASSES = ALLOWLISTED_CLASSES = {'a'}\n")]))
    reporters.append(dict(id='parse-error',check=CHECK,files=[dict(path='scripts/a.py',source="ALLOWLISTED_CLASSES = {'a'}\ndef broken(:\n")]))
    for name,raw in {'forward':[[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/a.py',count=1)],[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/b.py',count=2)]],
        'reverse':[[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/b.py',count=2)],[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/a.py',count=1)]],
        'last-zero':[[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/a.py',count=2),dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/b.py',count=0)]],
        'empty':[], 'non-owner':[[dict(variable_name='ALLOWLISTED_CLASSES',path='scripts/other.py',count=3)]],
    }.items():add('ordered-facts-'+name,'reporter_facts',[],dict(root=dict(path='.'),files=raw))
    for filename,cases in [('rr-fresh-grammar-allowlist-burndown-cases.json',api),('rr-grammar-allowlist-burndown-reporter-cases.json',reporters),('rr-fresh-grammar-allowlist-burndown-report-aggregation-cases.json',report_aggregation_cases())]:
        (FIXTURES/filename).write_text(json.dumps(dict(cases=cases),indent=2)+'\n');print(filename,len(cases))



def report_aggregation_cases():
    result=[]
    def add(name,sources):
        files=[dict(path='scripts/'+owner,source=''.join(sources.get(owner,[]))) for owner in ['lint_grammar_render_methods.py','lint_sql_grammar_escape_hatches.py']]
        result.append(dict(id=name,check=CHECK,entry='cli',files=files,arguments=dict(root=dict(path='.'),args=['--verbose'])))
    shared={}
    for i,(name,owner) in enumerate(zip(VARS,OWNERS)):shared.setdefault(owner,[]).append(name+f" = {{'common', 'unique-{i}'}}\n")
    add('all-five-share-an-entry',shared)
    varying={}
    for i,(name,owner) in enumerate(zip(VARS,OWNERS)):varying.setdefault(owner,[]).append(name+' = frozenset({'+','.join(repr(str(j)) for j in range(i))+'})\n')
    add('all-five-have-different-counts',varying)
    add('overlapping-assignment-union',{'lint_grammar_render_methods.py':["ALLOWLISTED_STATIC_CLASSES = {'a', 'b'}\nALLOWLISTED_STATIC_CLASSES = {'b', 'c'}\n"]})
    add('repeated-target-deduplication',{'lint_grammar_render_methods.py':["ALLOWLISTED_STATIC_CLASSES = ALLOWLISTED_STATIC_CLASSES = {'a', 'b'}\n"]})
    return result

if __name__=='__main__':main()
