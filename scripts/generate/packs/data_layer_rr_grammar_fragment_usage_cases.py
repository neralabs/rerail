"""Source-evaluated fragment cases: captures, aliases, scope, and traversal order."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
FIXTURES=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
CHECK='grammar_fragment_usage'


def grammar(value="'SELECT 1'", returned='self.sql', signature='self', init=None, name='SqlCapture'):
    body=init if init is not None else 'self.sql = '+value
    return f'class {name}:\n    def __init__({signature}):\n'+''.join('        '+line+'\n' for line in body.splitlines())+'    def render(self):\n        return '+returned+'\n'


def main():
    scenarios=[]
    def add(name,source):scenarios.append((name,[dict(path='case.py',source=source)]))
    for name,value in {
        'literal':"'SELECT 1'",'bytes':"b'SELECT 1'",'formatted':"f'SELECT {value}'",'tuple':"(42, 'SQL')",'set':"{'SQL'}",'list':"[0, 'SQL']",
        'dict-value':"{42: 'SQL'}",'dict-key-ignored':"{'SQL': 42}",'dict-splat':"{**'SQL'}",'addition':"other + 'SQL'",'subtraction-ignored':"other - 'SQL'",
        'if-body':"'SQL' if condition else 42",'if-else':"42 if condition else 'SQL'",'if-condition-ignored':"1 if 'SQL' else 2",'str-call':'str(value)',
        'str-attribute-ignored':'module.str(value)','str-alias-ignored':'string(value)','method-name-receiver-ignored':'value.lower()',
        'method-args-ignored':"value.format('SQL')",'call-container-ignored':"list(['SQL'])",'fstring-no-fields':"f'SQL'",'unicode-literal':"'é λ'",
    }.items():add('build-'+name,grammar(value))
    for method in ['casefold','format','format_map','join','lower','lstrip','removeprefix','removesuffix','replace','rstrip','strip','translate','upper','unknown']:
        add('method-'+method,grammar("'SQL'."+method+'(value)'))
    for name,returned in {
        'direct':'self.sql','class-qualified':'SqlCapture.sql','other-receiver':'other.sql','addition':'value + self.sql',
        'conditional-body':'self.sql if condition else value','conditional-else':'value if condition else self.sql',
        'conditional-test-ignored':'value if self.sql else other','subscript':'self.sql[0]','subscript-index-ignored':'other[self.sql]',
        'constructor':'SqlFragment(self.sql)','static':'SqlFragment.static(self.sql)','params':'SqlFragment.with_params(self.sql, values)',
        'bound-parameter-ignored':'SqlFragment.param(self.sql)','other-factory-ignored':'module.SqlFragment(self.sql)',
        'keyword-ignored':'SqlFragment(value=self.sql)','second-argument-ignored':'SqlFragment(42, self.sql)','starred-first-ignored':'SqlFragment(*self.sql)',
        'list-ignored':'[self.sql]','tuple-ignored':'(self.sql,)','subtract-ignored':'self.sql - other','unknown-call-ignored':'wrapper(self.sql)',
    }.items():add('return-'+name,grammar(returned=returned))
    for i,annotation in enumerate(['SqlFragment','pkg.SqlFragment','list[SqlFragment]','tuple[int, SqlFragment]','Callable[[SqlFragment], int]','SqlFragment | None','SqlFragment + int','"SqlFragment"','"list[SqlFragment]"','"NotSqlFragment"','"SqlFragment_"','"9SqlFragment"','"éSqlFragmentλ"','factory(SqlFragment)','[SqlFragment]','{SqlFragment}','{"a": SqlFragment}','Any','str','typing.Any','typing.str','"str"','str[int]','str()']):
        add('annotation-'+str(i),grammar(signature='self, value: '+annotation,init='self.sql = value'))
    for name,signature in {'posonly':'self, value: SqlFragment, /','vararg':'self, *value: SqlFragment','kwonly':'self, *, value: SqlFragment','kwarg':'self, **value: SqlFragment','self-skipped':'self: SqlFragment, value: int','untyped':'self, value','unsafe-str':'self, value: str','unsafe-any':'self, value: Any'}.items():add('parameter-'+name,grammar(signature=signature,init='self.sql = value'))
    for name,body in {
        'local-alias':"selected = 'SQL'\nself.sql = selected",'forward-local':"self.sql = selected\nselected = 'SQL'",'sticky-local':"selected = 'SQL'\nselected = 0\nself.sql = selected",'sticky-field':"self.sql = 'SQL'\nself.sql = 0",
        'bfs-alias-acquired-later':"if condition:\n    selected = 'SQL'\nself.sql = selected",'bfs-alias-before-deep':"if condition:\n    self.sql = selected\nselected = 'SQL'",'nested-function-leaks':"def nested():\n    selected = 'SQL'\n    self.sql = selected",'destructure':"self.sql, other = ('SQL', 1)",'destructure-short':"self.sql, other = ('SQL',)",'destructure-second':"other, self.sql = (1, 'SQL')",'destructure-not-string':"self.sql, other = (1, 'SQL')",'annotated-field':"self.sql: int = 'SQL'",'annotation-only':"self.sql: str",'chained-fields':"self.sql = self.other = 'SQL'",
    }.items():add('init-'+name,grammar(init=body))
    for name,prefix in {'literal':"RAW = 'SQL'\n",'alias':"RAW0 = 'SQL'\nRAW = RAW0\n",'sticky':"RAW = 'SQL'\nRAW = 0\n",'function-leak':"def outside():\n    RAW = 'SQL'\n",'annotated':"RAW: int = 'SQL'\n",'annotation-only':"RAW: str\n"}.items():add('module-'+name,prefix+grammar(value='RAW'))
    add('module-later-write',grammar(value='RAW')+"RAW = 'SQL'\n")
    add('module-name-shadowed-param',"RAW = 'SQL'\n"+grammar(signature='self, RAW: int',value='RAW'))
    for name,statement in {'value':"sql = 'SQL'",'str-annotation':'sql: str','attribute-str':'sql: builtins.str','quoted-str':"sql: 'str'",'container-str':'sql: list[str]','alias-does-not-propagate':"first = 'SQL'\n    sql = first",'module-alias':'sql = RAW'}.items():add('class-'+name,"RAW = 'SQL'\nclass SqlCapture:\n    "+statement+'\n    def render(self): return self.sql\n')
    fragment='class SqlCapture:\n    value: Fragment\n'
    for name,prefix in {
        'import': 'from p import SqlFragment as Fragment\n','assignment':'Fragment = SqlFragment\n','chain':'First = SqlFragment\nFragment = First\n',
        'sticky-assignment':'Fragment = SqlFragment\nFragment = int\n','reset-import':'Fragment = SqlFragment\nfrom p import Other as Fragment\n',
        'reset-then-write':'Fragment = SqlFragment\nfrom p import Other as Fragment\nFragment = SqlFragment\n',
        'different-grammar':'Fragment = SqlBase\n','canonical-rebound':'from p import Other as SqlFragment\nFragment = SqlFragment\n',
        'import-duplicate-last':'from p import SqlFragment as Fragment, Other as Fragment\n','import-duplicate-first':'from p import Other as Fragment, SqlFragment as Fragment\n',
        'nested-import-leaks':'class Ordinary:\n    from p import SqlFragment as Fragment\n','function-assignment-leaks':'def outside():\n    Fragment = SqlFragment\n',
        'class-assignment-ignored':'class Ordinary:\n    Fragment = SqlFragment\n',
    }.items():add('fragment-alias-'+name,prefix+fragment)
    for name,prefix in {'raw-prefix':'','import-prefix':'from p import SqlBase as Base\n','assignment':'Base = SqlBase\n','chain':'First = SqlBase\nBase = First\n','reset':'Base = SqlBase\nfrom p import Other as Base\n','known-class':'class Original:\n    def render(self): pass\nBase = Original\n','forward-class':'Base = Original\nclass Original:\n    def render(self): pass\n'}.items():
        base='SqlBase' if name=='raw-prefix' else 'Base';add('grammar-base-'+name,prefix+f'class Captured({base}):\n    value: SqlFragment\n')
    add('grammar-base-attribute','class Captured(pkg.SqlBase):\n    value: SqlFragment\n')
    add('grammar-base-subscript-ignored','class Captured(SqlBase[int]):\n    value: SqlFragment\n')
    add('known-class-sticky','class Base:\n    def render(self): pass\nclass Base: pass\nclass Captured(Base):\n    value: SqlFragment\n')
    add('async-init',grammar().replace('    def __init__','    async def __init__'))
    add('async-render',grammar().replace('    def render','    async def render'))
    add('first-init','class G:\n    def __init__(self): self.sql = 1\n    def __init__(self): self.sql = "SQL"\n    def render(self): return self.sql\n')
    add('first-render','class G:\n    sql = "SQL"\n    def render(self): return 1\n    def render(self): return self.sql\n')
    add('duplicate-helper-last','class G:\n    sql = "SQL"\n    def helper(self): return 1\n    def helper(self): return self.sql\n    def render(self): return self.helper()\n')
    add('helper-cycle','class G:\n    sql = "SQL"\n    def a(self): return self.b()\n    def b(self):\n        self.a()\n        return self.sql\n    def render(self): return self.a()\n')
    add('unreachable-helper','class G:\n    sql = "SQL"\n    def helper(self): return self.sql\n    def render(self): return 1\n')
    add('cls-reach','class G:\n    sql = "SQL"\n    def helper(cls): return G.sql\n    def render(self): return cls.helper()\n')
    add('nested-returns','class G:\n    sql = "SQL"\n    def render(self):\n        def nested(): return self.sql\n        return 1\n')
    add('duplicate-returns-one-line',grammar(returned='self.sql; return self.sql'))
    add('forbidden','class SqlDeferredRenderable:\n    value: SqlFragment\n    def render(self): return self.value\n')
    add('sqlfragment-self-capture',grammar(name='SqlFragment'))
    add('no-grammar','class Ordinary:\n    value: SqlFragment\n')
    add('nested-class-in-function','def outside():\n    class SqlInner:\n        value: SqlFragment\n        def __init__(self, arg: SqlFragment): pass\n')
    add('nested-init-definition','class SqlX:\n    def method(self):\n        def __init__(value: SqlFragment): pass\n')
    add('unicode-build-method',grammar("'SQL'.ｌｏｗｅｒ()"))
    add('same-line-parameter-order',grammar(signature='self, z: SqlFragment, a: SqlFragment',init='pass',returned='1'))
    add('unicode-fragment-alias','from p import SqlFragment as 片段\nclass SqlX:\n    value: "list[片段]"\n')
    add('string-word-boundary-delimiter',"class SqlX:\n    value: 'x SqlFragment x'\n")
    api=[dict(id=name,check=CHECK,entry='check_file',files=files,arguments=dict(file_path=dict(path='case.py'))) for name,files in scenarios]
    reporters=[dict(id=name,check=CHECK,files=[dict(f,path='impl/common/l2070_server_plan/'+f['path']) for f in files]) for name,files in scenarios]
    for path in ['impl/case.py','api/case.py','data_layer/case.py','_internal/case.py','l1x/case.py','l42/case.py','other/case.py','impl/tests/case.py','impl/test/case.py','impl/tmp/case.py','impl/.venv/case.py','impl/__pycache__/case.py']:
        reporters.append(dict(id='scope-'+path,check=CHECK,files=[dict(path=path,source=grammar())]))
    for path in ['tests/case.py','test/case.py','tmp/case.py','.venv/case.py']:
        api.append(dict(id='explicit-'+path,check=CHECK,entry='check_file',files=[dict(path=path,source=grammar())],arguments=dict(file_path=dict(path=path))))
    api.append(dict(id='directory-scope',check=CHECK,entry='check_directory',files=[dict(path=path,source=grammar()) for path in ['case.py','tests/a.py','test/b.py','.venv/c.py','tmp/d.py']],arguments=dict(directory=dict(path='.'))))
    for filename,cases in [('rr-fresh-grammar-fragment-usage-cases.json',api),('rr-grammar-fragment-usage-reporter-cases.json',reporters)]:
        (FIXTURES/filename).write_text(json.dumps(dict(cases=cases),indent=2)+'\n');print(filename,len(cases))

if __name__=='__main__':main()
