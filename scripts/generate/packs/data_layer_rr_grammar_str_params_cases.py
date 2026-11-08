"""Retain the scanner's textual annotations, slot policy, traversal order, and plugin scope."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];FIXTURES=ROOT/'gate/data-layer/fixtures/host/data-layer-port';CHECK='grammar_str_params'

def source(annotation='str',body='return str(self._value)',name='G',base='SqlBase',signature=None,init=True,render=True):
    text=f'class {name}'+(f'({base})' if base else '')+':\n'
    if init:text+='    def __init__('+(signature or f'self, value: {annotation}')+'):\n        pass\n'
    if render:text+='    def render(self):\n'+''.join('        '+line+'\n' for line in body.splitlines())
    return text


def main():
    snippets={}
    annotations=['str','str | None','None | str','int | str | bytes','str | (int | bytes)','int | (str | bytes)',
        '(int | str) | bytes','list[str]','typing.Optional[str]','Annotated[str, "tag"]','"str"','"left|str|right"',
        'Literal["left|str|right"]','Literal["str"]','Any','SqlIdentifier','strange','Str','Ｓｔｒ','ｓｔｒ','str or int',
        'str if flag else int','str | Callable[[int], bytes]','(lambda: str)','None','None | "str"','str | (lambda: int)',
        '(str, int)','tuple[str | None]','str | 1e309','str | b"x"','str | 0xffffffffffffffffffffffff']
    for i,annotation in enumerate(annotations):snippets['annotation-'+str(i)]=source(annotation)
    for name,signature in {
        'ordinary':'self, value: str','self-str-skipped':'self: str, value: SqlIdentifier','self-posonly':'self, /, value: str',
        'posonly-excluded':'self, value: str, /','mixed-posonly':'self, ignored: str, /, value: str',
        'kwonly-excluded':'self, *, value: str','vararg-excluded':'self, *value: str','kwarg-excluded':'self, **value: str',
        'positional-cls-flagged':'cls: str, value: str','untyped-init-empty':'self, value',
        'multiple-params':'self, value: str, other: str','default-value':'self, value: str = "v"',
    }.items():snippets['signature-'+name]=source(signature=signature)
    for name,body in {
        'call':'return str(self._value)','duplicates':'return str(self._value, self._value)',
        'two-calls':'return str(self._value), str(self._value)','concat':'return str(self._value) + str(self._value)',
        'concat-three':'return str(self._value) + str(self._value) + str(self._value)',
        'concat-multiple-args':'return str(self._value, self._value) + str(self._value)',
        'subtraction':'return str(self._value) - str(self._value)','nested-call':'return wrapper(str(self._value)) + "x"',
        'call-dotted':'return builtins.str(self._value)','call-parentheses':'return (str)((self)._value)',
        'call-keyword':'return str(value=self._value)','call-starred':'return str(*self._value)',
        'call-kwstar':'return str(**self._value)','call-nested-slot':'return str(self._value.other)',
        'call-other-receiver':'return str(other._value)','call-nonunderscore':'return str(self.value)',
        'call-other-slot':'return str(self._other)','keyword-before-starred':'return str(value=f"{x}", *[self._value])',
        'nested-helper':'def helper():\n    return str(self._value)\nreturn helper()',
        'nested-lambda':'return lambda: str(self._value)',
        'nested-nongrammar':'class Inner:\n    def helper(self):\n        return str(self._value)\nreturn None',
        'nested-grammar':'class Inner(SqlBase):\n    def render(self):\n        return str(self._other)\nreturn str(self._value)',
        'dict-field-order':'return {0: str(self._value), str(self._value): 1}',
        'conditional-field-order':'return str(self._value) if str(self._value) else "x"',
        'unicode-slot':'return ｓｔｒ(ｓｅｌｆ._ｖａｌｕｅ)',
    }.items():snippets[name]=source(body=body)
    snippets['no-init-wildcard']=source(init=False,body='return str(self.public), str(self._unknown)')
    snippets['typed-init-excludes']=source(annotation='SqlIdentifier',body='return str(self._value), str(self._unknown)')
    snippets['empty-init-excludes']=source(signature='self',body='return str(self._unknown)')
    snippets['async-init-wildcard']=source(body='return str(self._value)').replace('    def __init__','    async def __init__')
    snippets['async-render-excluded']=source().replace('    def render','    async def render')
    snippets['only-async-render-no-base']=source(base='',init=False).replace('    def render','    async def render')
    snippets['decorated-render']=source().replace('    def render','    @decorator(str(self._value))\n    def render')
    snippets['render-default']=source().replace('def render(self):','def render(self, x=str(self._value)):')
    snippets['render-return-annotation']=source().replace('def render(self):','def render(self) -> str(self._value):')
    snippets['render-before-init']='class G(SqlBase):\n    def render(self):\n        return str(self._value)\n    def __init__(self, value: str): pass\n'
    snippets['two-inits']='class G(SqlBase):\n    def __init__(self, value: str): pass\n    def __init__(self, value: str): pass\n    def render(self): return str(self._value)\n'
    snippets['nested-init-ignored']='class G(SqlBase):\n    if condition:\n        def __init__(self, value: str): pass\n    def render(self): return str(self._value)\n'
    snippets['nested-render-not-grammar']='class G:\n    def __init__(self, value: str): pass\n    if condition:\n        def render(self): return str(self._value)\n'
    for name in ['SqlQuotedIdentifier','SqlFragment','SqlTypeRef','ViewDefinition','SqlExplain']:
        snippets['class-allowlist-'+name]=source(name=name)
    snippets['param-allowlist']='class SqlCreateTriggerStmt(SqlBase):\n    def __init__(self, timing: str, event: str, value: str): pass\n    def render(self): return str(self._timing), str(self._event), str(self._value)\n'
    snippets['param-allowlist-without-init']=source(name='SqlCreateTriggerStmt',init=False,body='return str(self._timing)')
    for base in ['SqlBase','_SqlBase','pkg.SqlBase','NotSql','SqlBase[T]','(SqlBase)','_SqlBase[T]']:
        snippets['base-'+base]=source(base=base,render=False)
    snippets['class-name-only']=source(name='SqlSomething',base='',render=False)
    snippets['render-fallback']=source(base='Unrelated')
    snippets['helper-method-excluded']=source().replace('def render(','def helper(')
    snippets['async-nested-inherits']=source(body='async def helper():\n    return str(self._value)\nreturn helper()')
    snippets['syntax-with-trigger']='class G(SqlBase):\n    def __init__(self, value: str:\n'
    snippets['bom']='\ufeff'+source()
    api=[];reporters=[]
    for name,text in snippets.items():
        api.append(dict(id=name,check=CHECK,entry='lint_file',files=[dict(path='impl/case.py',source=text)],arguments=dict(filepath=dict(path='impl/case.py'))))
        reporters.append(dict(id=name,check=CHECK,files=[dict(path='impl/common/l2100_sql_grammar/case.py',source=text)]))
    for name,path in [('test-folder','impl/test/case.py'),('tests-folder','impl/tests/case.py'),('test-filename','impl/test_case.py'),('dunder-filename','impl/__init__.py'),('wrong-extension','impl/case.pyi')]:
        api.append(dict(id=name,check=CHECK,entry='lint_file',files=[dict(path=path,source=source())],arguments=dict(filepath=dict(path=path))))
    for name,provided,actual in [('gate-empty','pass\n',source()),('gate-present','str',source()),('actual-empty','str','pass\n')]:
        api.append(dict(id=name,check=CHECK,entry='lint_source',files=[dict(path='impl/case.py',source=actual)],arguments=dict(filepath=dict(path='impl/case.py'),source=provided)))
    api.append(dict(id='missing-file',check=CHECK,entry='lint_file',files=[],arguments=dict(filepath=dict(path='impl/missing.py'))))
    api.append(dict(id='invalid-utf8',check=CHECK,entry='lint_file',files=[dict(path='impl/case.py',bytesHex='ff')],arguments=dict(filepath=dict(path='impl/case.py'))))
    for path in ['impl/outside.py','impl/common/l2100_sql_grammar/test/case.py','impl/common/l2100_sql_grammar/tests/case.py','impl/common/l2100_sql_grammar/test_case.py','impl/common/l2100_sql_grammar/__init__.py']:
        reporters.append(dict(id='scope-'+path,check=CHECK,files=[dict(path=path,source=source())]))
    reporters.append(dict(id='multi-file-sorted',check=CHECK,files=[dict(path='impl/common/l2100_sql_grammar/z.py',source=source()),dict(path='impl/common/l2100_sql_grammar/a.py',source=source(body='return str(self._value) + str(self._value)'))]))
    grammar='impl/common/l2100_sql_grammar/'
    for name,args,files in [
        ('empty',[],[]),('check-positive',['--check'],[dict(path=grammar+'case.py',source=source())]),
        ('report-positive',[],[dict(path=grammar+'case.py',source=source())]),
        ('summary-duplicates',['--check','--summary'],[dict(path=grammar+'case.py',source=source(body='return str(self._value, self._value) + str(self._value)'))]),
        ('shallow-discovery',['--check'],[dict(path=grammar+'test_case.py',source=source()),dict(path=grammar+'__init__.py',source=source()),dict(path=grammar+'nested/case.py',source=source())]),
    ]:
        api.append(dict(id='cli-'+name,check=CHECK,entry='cli',files=files,arguments=dict(root=dict(path='.'),args=args)))
    for filename,cases in [('rr-fresh-grammar-str-params-cases.json',api),('rr-grammar-str-params-reporter-cases.json',reporters)]:
        (FIXTURES/filename).write_text(json.dumps(dict(cases=cases),indent=2,ensure_ascii=True)+'\n');print(filename,len(cases))

if __name__=='__main__':main()
