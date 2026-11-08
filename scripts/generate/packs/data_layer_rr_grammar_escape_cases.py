"""Supplement original grammar tests with source-derived annotations, scope and ordering cases."""
import argparse
import json
from pathlib import Path


def build():
    cases=[]
    annotations=['str','object','Any','int','bytes','SqlTableName','SqlTableName | None','list[str]',
        'dict[SqlTableName, tuple[str, int]]','list','Optional[SqlTableName]','typing.Optional[str]',
        'typing.SqlTableName',"'list[str]'","'typing.Optional[SqlTableName]'","''","'𝓐'",'𝓐',
        "b'raw'","b'\\xff'","list[b'raw']",'1.25','1_000.00','1e2','2e309',
        '0x10','123456789012345678901234567890','0x123456789012345678901234567890','1j','2.5j','...',
        'True','None','factory(str)','1 + 2','str | str']
    def scan(identity,source,extra=None):
        args=dict(file_path={'path':'impl/case.py'});args.update(extra or {})
        cases.append(dict(id=identity,check='sql_grammar_escape_hatches',entry='scan_file',
            files=[dict(path='impl/case.py',source=source)],arguments=args))
    for i,annotation in enumerate(annotations):
        scan('annotation-'+str(i),'class Fragment(SqlRenderable):\n    def __init__(self, value: '+annotation+'):\n        pass\n')
    for identity,source in [
        ('posonly','class Fragment(SqlRenderable):\n    def __init__(self, value: str, /): pass\n'),
        ('kwonly','class Fragment(SqlRenderable):\n    def __init__(self, *, value: str): pass\n'),
        ('varargs','class Fragment(SqlRenderable):\n    def __init__(self, *value: str, **other: Any): pass\n'),
        ('async-init','class Fragment(SqlRenderable):\n    async def __init__(self, value: str): pass\n'),
        ('same-line-dedupe','class Fragment(SqlRenderable):\n    def __init__(self, first: str, second: object): pass\n'),
        ('class-usage-dedupe','class SqlRawSql(SqlRenderable): pass\n'),
        ('alias-stages','Alias = SqlRenderable\nclass Derived(Alias):\n    def __init__(self, v: str): pass\nclass Other(SqlRenderable):\n    def __init__(self, v: Derived): pass\n'),
        ('inheritance','class Parent(SqlRenderable): pass\nclass Child(Parent):\n    def __init__(self, value: str): pass\n'),
        ('alias-union','class Parent(SqlRenderable): pass\nAlias = Parent | int\nclass Child(Alias):\n    def __init__(self, value: str): pass\n'),
        ('protected','class SqlAdminToolRawStatement(SqlRenderable):\n    def __init__(self, value: str): pass\n'),
        ('nested','class Outer(SqlRawSql):\n    class Nested(SqlRenderable):\n        def __init__(self, value: str): pass\n'),
        ('syntax','class Broken(SqlRenderable):\n    def __init__(self, x: str)\n'),
        ('malformed-heuristic',"def f(x='(:'):\n    class Fragment(SqlRenderable):\n        def __init__(self, value: str): pass\n"),
        ('no-trigger-syntax','unrelated broken )\n')]:scan(identity,source)
    source='class Fragment(SqlRenderable):\n    def __init__(self, value: str): pass\n'
    for i,values in enumerate([dict(grammar_class_names={'frozenset':[]},safe_param_types={'frozenset':['str']}),
            dict(grammar_class_names={'frozenset':['SqlRenderable']},safe_param_types={'frozenset':['str']}),
            dict(grammar_class_names={'frozenset':[]}),dict(safe_param_types={'frozenset':['str']})]):
        scan('override-'+str(i),source,values)
    usage_sources=['x = SqlRawSql()\n',"text = 'SqlRawSql()'\n",'import SqlRawSqlWithParams\n',
        '\u00a0\x1c# SqlRawSql()\n','broken ) SqlRawSql(\n','x = SqlRawSql()\rSqlUnsafe()\n']
    for i,source in enumerate(usage_sources):
        cases.append(dict(id='usage-'+str(i),check='sql_grammar_escape_hatches',entry='scan_usages',
            files=[dict(path='impl/case.py',source=source)],arguments=dict(file_path={'path':'impl/case.py'})))
    for i,paths in enumerate([[{'path':'impl/a.py'},{'path':'impl/b.py'}],[{'path':'.'}],[{'path':'impl/tests'}]]):
        files=[dict(path='impl/a.py',source='class Parent(SqlRenderable): pass\n'),
            dict(path='impl/b.py',source='# SqlRenderable\nclass Child(Parent):\n    def __init__(self, value: str): pass\n'),
            dict(path='impl/tests/usage.py',source='x = SqlRawSql()\n')]
        cases.append(dict(id='collect-'+str(i),check='sql_grammar_escape_hatches',entry='_collect_violations',files=files,arguments=dict(paths=paths)))
    for entry in ['scan_file','scan_usages']:
        cases.append(dict(id='missing-'+entry,check='sql_grammar_escape_hatches',entry=entry,files=[],arguments=dict(file_path={'path':'missing.py'})))
    return dict(format='rerail.grammar-escape-api-cases/v1',cases=cases)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);p.add_argument('--reporters',type=Path);a=p.parse_args()
    data=build();a.out.write_text(json.dumps(data,indent=2,ensure_ascii=True)+'\n')
    if a.reporters:
        a.reporters.write_text(json.dumps(dict(cases=reporter_cases()),indent=2)+'\n')
    print(json.dumps(dict(cases=len(data['cases']),reporters=len(reporter_cases()))))


def reporter_cases():
    result=[]
    def classes(count,base='SqlRenderable',annotation='str',twice=False):
        return ''.join('class Fragment'+str(i)+'('+base+'):\n    def __init__(self, value: '+annotation+(', other: object' if twice else '')+'): pass\n' for i in range(count))
    for count in [0,1,16,17,18]:
        result.append(dict(id='ceiling-'+str(count),check='sql_grammar_escape_hatches',files=[dict(path='impl/case.py',source=classes(count))]))
    result.append(dict(id='dedupe-before-ceiling',check='sql_grammar_escape_hatches',files=[dict(path='impl/case.py',source=classes(16,twice=True))]))
    result.append(dict(id='dedupe-reasons',check='sql_grammar_escape_hatches',files=[dict(path='impl/case.py',source=classes(17,twice=True))]))
    result.append(dict(id='cross-file-inheritance',check='sql_grammar_escape_hatches',files=[
        dict(path='impl/base.py',source='class Parent(SqlRenderable): pass\n'),
        dict(path='api/child.py',source='# SqlRenderable\n'+classes(17,base='Parent'))]))
    result.append(dict(id='no-trigger-child',check='sql_grammar_escape_hatches',files=[
        dict(path='impl/base.py',source='class Parent(SqlRenderable): pass\n'),
        dict(path='api/child.py',source=classes(17,base='Parent'))]))
    for i,annotation in enumerate(['1.25',"list[b'\\xff']",'123456789012345678901234567890']):
        result.append(dict(id='constant-'+str(i),check='sql_grammar_escape_hatches',files=[dict(path='impl/case.py',source=classes(17,annotation=annotation))]))
    for i,path in enumerate(['impl/tests/a.py','impl/test/a.py','impl/tmp/a.py','other/a.py','api/a.py']):
        result.append(dict(id='scope-'+str(i),check='sql_grammar_escape_hatches',files=[dict(path=path,source=classes(17))]))
    return result


if __name__=='__main__':main()
