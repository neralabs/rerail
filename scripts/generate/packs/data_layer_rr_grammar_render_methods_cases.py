"""Retain source-derived render contexts, event order, lexical details, and plugin scope cases."""
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
FIXTURES=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
CHECK='grammar_render_methods'

def wrapped(body,base='SqlRenderable',name='Fragment',method='render'):
    return f'class {name}({base}):\n    def {method}(self):\n'+''.join('        '+line+'\n' for line in body.splitlines())


def main():
    snippets={}
    def add(name,body,**kw):snippets[name]=wrapped(body,**kw)
    for name,body in {
        'f-string': 'return f"select {value}"',
        'f-dedupe': 'return f"{x}" + f"{y}"',
        'f-implicit': 'return "head" f"{x}" "tail"',
        'f-format-spec': 'return f"{x:{width}.{precision}f}"',
        'f-multiline-spec': 'return f"""{x:\n{width}}"""',
        'exception-direct': 'raise GrammarException(f"{x}")',
        'exception-nested': 'raise GrammarException(wrapper(f"{x}"))',
        'exception-one-attr': 'raise errors.GrammarException(f"{x}")',
        'exception-deep-attr': 'raise pkg.errors.GrammarException(f"{x}")',
        'exception-receiver-call': 'raise factory().GrammarException(f"{x}")',
        'exception-format': 'raise GrammarException(f"{x:{width}}")',
        'exception-other-violation': 'raise GrammarException(SF.static(x), f"{x}")',
        'static-runtime': 'return SqlFragment.static(value)',
        'static-bare': 'return static(value)',
        'static-deep-attr': 'return pkg.SqlFragment.static(value)',
        'static-unrelated': 'return other.static(value)',
        'static-receiver-call': 'return factory().static(value)',
        'static-many-args': 'return SF.static(first, second)',
        'static-leading-literal': 'return SF.static("literal", value)',
        'static-keyword': 'return SF.static(value=payload)',
        'static-starred': 'return SF.static(*values)',
        'static-kwstar': 'return SF.static(**values)',
        'static-number': 'return SF.static(42)',
        'static-bytes': 'return SF.static(b"literal")',
        'static-none': 'return SF.static(None)',
        'static-constant': 'return SF.static("a" "b")',
        'static-parentheses': 'return SF.static(((value)))',
        'static-fstring': 'return SF.static(f"{value}")',
        'static-unicode-source': 'return SF.static(α.β + \'é\')',
        'static-multiline': 'return SF.static(\n    (left +\n     right)\n)',
        'join-string': 'return ", ".join(values)',
        'join-bytes': 'return b", ".join(values)',
        'join-number': 'return (42).join(values)',
        'join-negative': 'return (-42).join(values)',
        'join-none': 'return None.join(values)',
        'join-ellipsis': 'return (...).join(values)',
        'join-fstring': 'return f"{x}".join(values)',
        'join-name': 'return sep.join(values)',
        'format-string': 'return "{}".format(value)',
        'format-name': 'return template.format(value)',
        'format-call': 'return factory().format(value)',
        'template-bare': 'return Template(value)',
        't-bare': 'return t(value)',
        'template-one-attr': 'return pkg.Template(value)',
        'template-deep-attr': 'return pkg.sub.Template(value)',
        'concat-strings': 'return "a" + "b"',
        'concat-fstrings': 'return f"{a}" + f"{b}"',
        'concat-one-name': 'return "a" + value',
        'multiply-strings': 'return "a" * "b"',
        'concat-parenthesized': 'return ("a") + (("b"))',
        'same-line-types': 'return SF.static(value), "{}".format(x), f"{x}", Template(y)',
        'same-line-static-first': 'a = SF.static(first); b = SF.static(second)',
        'helper-function': 'def helper():\n    return f"{value}"\nreturn helper()',
        'lambda': 'return lambda: f"{value}"',
        'nested-non-grammar': 'class Inner:\n    def method(self):\n        return f"{value}"\nreturn Inner()',
        'nested-grammar': 'class Inner(SqlRenderable):\n    def render(self):\n        return f"{value}"\nreturn f"{other}"',
        'nested-render-default': 'def render(self, value=f"{x}"):\n    return "ok"\nreturn "done"',
        'nested-function-decorator': '@decorator(f"{x}")\ndef helper():\n    return None\nreturn "ok"',
        'missing-trigger': 'return f"{x}"',
    }.items():add(name,body)
    snippets['missing-trigger']=wrapped('return f"{x}"').replace('render(', 'render (')
    for base in ['ns.SqlRenderable','SqlRenderable[T]','SqlContainer[int][str]','NotSqlish','NotRelated','sqlRenderable','(SqlRenderable)']:
        add('base-'+base,'return f"{x}"',base=base)
    add('nonrender-method','return f"{x}"',method='execute')
    snippets['top-level-function']='def render():\n    return f"{x}"\n'
    snippets['nested-in-helper']='class Fragment(SqlRenderable):\n    def helper(self):\n        def render():\n            return f"{x}"\n        return render()\n'
    snippets['async-render']=wrapped('return SF.static(value)').replace('    def render(', '    async def render(')
    snippets['decorated-class']='@decorator\n'+wrapped('return SF.static(value)')
    snippets['decorated-render']=wrapped('return SF.static(value)').replace('    def render', '    @decorator(f"{x}")\n    def render')
    snippets['unicode-names']=wrapped('return ＳＦ.static(value)',base='ＳｑｌRenderable',name='Ｆragment',method='ｒｅｎｄｅｒ')
    snippets['splitlines-details']=wrapped('x = """a\u2028b"""\nreturn ",".join(values)')
    api=[];reporters=[]
    for name,source in snippets.items():
        api.append(dict(id=name,check=CHECK,entry='scan_file',files=[dict(path='impl/case.py',source=source)],arguments=dict(file_path=dict(path='impl/case.py'))))
        reporters.append(dict(id=name,check=CHECK,files=[dict(path='impl/common/l2100_sql_grammar/case.py',source=source)]))
    for name,path in [('tests-folder','impl/tests/case.py'),('test-folder','impl/test/case.py'),('test-filename','impl/test_case.py'),('wrong-extension','impl/case.pyi')]:
        api.append(dict(id=name,check=CHECK,entry='scan_file',files=[dict(path=path,source=wrapped('return SF.static(value)'))],arguments=dict(file_path=dict(path=path))))
    for name,source in [('syntax-error','class Broken(\n'),('empty-source',''),('unrelated-source','value = 42\n')]:
        api.append(dict(id=name,check=CHECK,entry='scan_file',files=[dict(path='impl/case.py',source=source)],arguments=dict(file_path=dict(path='impl/case.py'))))
    api.append(dict(id='scan-source-ignores-source-and-tree',check=CHECK,entry='scan_source',files=[dict(path='impl/case.py',source=wrapped('return SF.static(value)'))],arguments=dict(file_path=dict(path='impl/case.py'),source='pass\n',tree=dict(pythonAst='pass\n',mode='exec'))))
    api.append(dict(id='unicode-read-failure',check=CHECK,entry='scan_file',files=[dict(path='impl/case.py',bytesHex='ff')],arguments=dict(file_path=dict(path='impl/case.py'))))
    for path in ['impl/outside.py','impl/common/l2100_sql_grammar/tests/case.py','impl/common/l2100_sql_grammar/test/case.py','impl/common/l2100_sql_grammar/test_case.py']:
        reporters.append(dict(id='scope-'+path,check=CHECK,files=[dict(path=path,source=wrapped('return SF.static(value)'))]))
    for name,value in [('api',dict(format='rerail.rr-fresh-contract-cases/v1',cases=api)),('reporter',dict(cases=reporters))]:
        file='rr-fresh-grammar-render-methods-cases.json' if name=='api' else 'rr-grammar-render-methods-reporter-cases.json'
        (FIXTURES/file).write_text(json.dumps(value,indent=2,ensure_ascii=True)+'\n');print(name,len(value['cases']))

if __name__=='__main__':main()
