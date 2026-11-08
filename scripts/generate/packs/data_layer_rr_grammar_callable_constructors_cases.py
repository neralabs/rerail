"""Exercise cross-file registries, source scope quirks, breadth-first field writes, and all finding codes."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];FIXTURES=ROOT/'gate/data-layer/fixtures/host/data-layer-port';CHECK='grammar_callable_constructors';GRAMMAR='impl/common/l2100_sql_grammar/'

def grammar(annotation='Callable',init_body='self._fn = callback',render_body='return self._fn()',signature=None,name='G'):
    return f'class {name}:\n    def __init__('+(signature or f'self, callback: {annotation}')+'):\n'+''.join('        '+line+'\n' for line in init_body.splitlines())+'    def render(self):\n'+''.join('        '+line+'\n' for line in render_body.splitlines())


def main():
    scenarios=[]
    def add(name,text):scenarios.append((name,[dict(path='case.py',source=text)]))
    for i,annotation in enumerate(['Callable','typing.Callable[[int], str]','list[Callable]','Callable | None','"Callable"','"left|Callable|right"','Any','typing.Any','Any[int]','Any()','Any | None','int','None','factory(Callable)','factory(Callable=int)','Literal["Callable"]','Annotated[int, Callable]','((Any))','((Callable))']):add('annotation-'+str(i),grammar(annotation))
    add('untyped',grammar(signature='self, callback'))
    for name,signature in {'posonly':'self, callback: Callable, /','kwonly':'self, *, callback: Callable','vararg':'self, *callback: Callable','kwarg':'self, **callback: Callable','self-skipped':'self: Callable, callback: int'}.items():add('parameter-'+name,grammar(signature=signature))
    for name,init_body in {
        'direct':'self._fn = callback','unrelated-value':'self._fn = other','constant-value':'self._fn = 42',
        'attribute-value':'self._fn = object.callback','annotated-assignment':'self._fn: Callable = callback',
        'chain':'self._fn = self._other = callback','duplicates-two-fields':'self._a = callback\nself._b = callback',
        'tuple-target':'self._fn, self._other = callback','other-receiver':'other._fn = callback',
        'last-shallow-write':'self._fn = callback\nself._fn = other',
        'bfs-deep-before-shallow':'if condition:\n    self._fn = other\nself._fn = callback',
        'bfs-shallow-before-deep':'self._fn = callback\nif condition:\n    self._fn = other',
        'bfs-nested-function':'def nested():\n    self._fn = other\nself._fn = callback',
        'bfs-nested-class':'class Nested:\n    self._fn = other\nself._fn = callback',
    }.items():add('field-'+name,grammar(init_body=init_body))
    for name,body in {'not-called':'return self._fn','deep-field':'return self._fn.render()','other-receiver':'return other._fn()',
        'parentheses':'return (self)._fn()','nested-helper':'def helper():\n    return self._fn()\nreturn helper()',
        'nested-class':'class Nested:\n    def run(self):\n        return self._fn()\nreturn Nested()',
        'return-call':'return self._fn()()'}.items():add('render-'+name,grammar(render_body=body))
    add('async-init',grammar().replace('    def __init__','    async def __init__'))
    add('async-render',grammar().replace('    def render','    async def render'))
    add('first-init-wins','class G:\n    def __init__(self, callback: int): self._fn = callback\n    def __init__(self, callback: Callable): self._fn = callback\n    def render(self): return self._fn()\n')
    add('first-render-wins','class G:\n    def __init__(self, callback: Callable): self._fn = callback\n    def render(self): return self._fn\n    def render(self): return self._fn()\n')
    add('nested-renderable-class','def outer():\n'+''.join('    '+line+'\n' for line in grammar().splitlines()))
    base='class G:\n    def render(self): return 1\n'
    fn='def callback(): pass\n'
    for name,expr in {'lambda':'G(lambda: 1)','function':'G(callback)','duplicate-function':'G(callback, callback)',
        'keyword':'G(value=callback)','kwstar-name':'G(**callback)','kwstar-lambda':'G(**(lambda: 1))',
        'star-name':'G(*callback)','attribute-value':'G(module.callback)','literal':'G(42)',
        'dotted-class':'pkg.G(callback)','subscript-class':'G[int](callback)','call-as-callee':'G()(callback)'}.items():add('constructor-'+name,base+fn+expr+'\n')
    for name,body,signature in [
        ('module-function','return G(callback)',''),('parameter-shadow','return G(callback)','callback'),
        ('assignment-shadow','callback = 42\nreturn G(callback)',''),('assignment-after-use','value = G(callback)\ncallback = 42\nreturn value',''),
        ('annotated-binding','callback: object\nreturn G(callback)',''),('augmented-binding','callback += other\nreturn G(callback)',''),
        ('loop-binding','for callback in values: pass\nreturn G(callback)',''),('async-loop-binding','async for callback in values: pass\nreturn G(callback)',''),
        ('tuple-binding','callback, other = values\nreturn G(callback)',''),('starred-binding-ignored','other, *callback = values\nreturn G(callback)',''),
        ('nested-direct-function','def callback(): pass\nreturn G(callback)',''),
        ('nested-function-beats-binding','def callback(): pass\ncallback = 42\nreturn G(callback)',''),
        ('nested-body-binding-leaks','def helper():\n    callback = 42\nreturn G(callback)',''),
        ('nested-parameter-does-not-leak','def helper(callback): pass\nreturn G(callback)',''),
        ('class-body-binding-leaks','class Nested:\n    callback = 42\nreturn G(callback)',''),
        ('class-method-binding-leaks','class Nested:\n    def method(self):\n        callback = 42\nreturn G(callback)',''),
        ('import-not-binding','from elsewhere import callback\nreturn G(callback)',''),
        ('walrus-not-binding','(callback := other)\nreturn G(callback)',''),
        ('sync-with-binding','with manager() as callback: pass\nreturn G(callback)',''),
        ('async-with-not-binding','async with manager() as callback: pass\nreturn G(callback)',''),
        ('lambda-no-scope','return lambda callback: G(callback)',''),
    ]:
        add('scope-'+name,base+fn+'def outer('+signature+'):\n'+''.join('    '+line+'\n' for line in body.splitlines()))
    add('async-function-no-scope',base+fn+'async def outer(callback):\n    return G(callback)\n')
    add('async-module-function-ignored',base+'async def callback(): pass\nG(callback)\n')
    add('module-assignment-ignored',base+fn+'callback = 42\nG(callback)\n')
    add('conditional-def-not-module-name',base+'if condition:\n    def callback(): pass\nG(callback)\n')
    add('nested-class-not-global-registry','def outer():\n    class Local:\n        def render(self): return 1\n    return Local(lambda: 1)\n')
    for name,expr in {'bare':'SimpleNamespace(render=lambda: 1)','attribute':'types.SimpleNamespace(render=None)',
        'generic':'SimpleNamespace[int](render=1)','keyword-unicode':'SimpleNamespace(ｒｅｎｄｅｒ=None)',
        'kwstar-ignored':'SimpleNamespace(**{"render": lambda: 1})','alias-ignored':'SN(render=lambda: 1)'}.items():add('namespace-'+name,expr+'\n')
    for name,text in {'assign':'obj.render = value','annotated':'obj.render: object','annotated-value':'obj.render: object = value',
        'chain':'left.render = right.render = value','tuple-ignored':'obj.render, other = values','augmented-ignored':'obj.render += value',
        'unrelated-ignored':'obj.run = value','unicode':'obj.ｒｅｎｄｅｒ = value'}.items():add('dynamic-'+name,text+'\n')
    scenarios.append(('cross-file-registry',[dict(path='a.py',source=base),dict(path='b.py',source=fn+'G(callback)\n')]))
    scenarios.append(('cross-file-function-not-visible',[dict(path='a.py',source=base+fn),dict(path='b.py',source='G(callback)\n')]))
    for path in ['tests/case.py','test/case.py','fragments.py','nested/fragments.py','test_case.py','.venv/case.py','tmp/case.py']:
        scenarios.append(('file-'+path,[dict(path=path,source='SimpleNamespace(render=None)\n')]))
    scenarios.append(('excluded-registry',[dict(path='tests/a.py',source=base),dict(path='b.py',source='G(lambda: 1)\n')]))
    scenarios.append(('fragment-registry-excluded',[dict(path='fragments.py',source=base),dict(path='b.py',source='G(lambda: 1)\n')]))
    scenarios.append(('syntax-error-registry',[dict(path='a.py',source=base+'invalid(\n'),dict(path='b.py',source='G(lambda: 1)\n')]))
    add('unicode',grammar(signature='self, ｃａｌｌｂａｃｋ: Callable').replace('callback','ｃａｌｌｂａｃｋ'))
    api=[dict(id=name,check=CHECK,entry='collect_violations',files=files,arguments=dict(root=dict(path='.'))) for name,files in scenarios]
    reporters=[dict(id=name,check=CHECK,files=[dict(path='scripts/lint_grammar_callable_constructors.py',source='')]+[dict(f,path=GRAMMAR+f['path']) for f in files]) for name,files in scenarios]
    reporters.append(dict(id='invalid-marker',check=CHECK,files=[dict(path='scripts/lint_grammar_callable_constructors.py',source='invalid('),dict(path=GRAMMAR+'case.py',source='SimpleNamespace(render=None)\n')]))
    reporters.append(dict(id='tests-workspace',check=CHECK,workspaceName='tests',files=[dict(path='scripts/lint_grammar_callable_constructors.py',source=''),dict(path=GRAMMAR+'case.py',source='SimpleNamespace(render=None)\n')]))
    reporters.append(dict(id='missing-marker',check=CHECK,files=[dict(path=GRAMMAR+'case.py',source='SimpleNamespace(render=None)\n')]))
    for name,args,files in [
        ('empty',['--check'],[]),('check-positive',['--check'],[dict(path=GRAMMAR+'case.py',source='SimpleNamespace(render=None)\n')]),
        ('report-positive',[],[dict(path=GRAMMAR+'case.py',source='obj.render = value\n')]),
        ('custom-root',['--check','--grammar-root','custom'],[dict(path='custom/case.py',source=grammar())]),
        ('excluded-files',['--check'],[dict(path=GRAMMAR+'tests/case.py',source=grammar()),dict(path=GRAMMAR+'fragments.py',source=grammar())]),
    ]:
        api.append(dict(id='cli-'+name,check=CHECK,entry='cli',files=files,arguments=dict(root=dict(path='.'),args=args)))
    for i,detail in enumerate(['ordinary detail',"quotes: ' and \"",'é λ \nsecond line']):
        api.append(dict(id='format-'+str(i),check=CHECK,entry='format',files=[],arguments=dict(path=dict(path='case.py'),line=i+1,code='CODE',detail=detail)))
    for filename,cases in [('rr-fresh-grammar-callable-constructors-cases.json',api),('rr-grammar-callable-constructors-reporter-cases.json',reporters)]:
        (FIXTURES/filename).write_text(json.dumps(dict(cases=cases),indent=2,ensure_ascii=True)+'\n');print(filename,len(cases))

if __name__=='__main__':main()
