"""Record complete composition source DTOs, traversal order, normalization and errors."""
from pathlib import Path
from dto import canonical

CHECK='plan_composition_architecture'


def cases(module):
    rows=[]
    def add(name,source):rows.append(dict(id=name,source=source))
    for part in module._FORBIDDEN_IMPORT_PARTS:
        for i,source in enumerate([f'import app.{part}',f'import app.{part} as alias',f'from app.{part} import X',
            f'from ..app.{part} import X',f'import os as operating, app.{part} as alias',f'from app.\\\n {part} import X']):
            add(f'import-{part}-{i}',source)
    for suffix in ['lazy_result','lazy_seq']:
        for source in [f'import x.{suffix}',f'from x.{suffix} import X',f'import {suffix}',f'import x.{suffix}, os']:
            add('suffix-'+str(len(rows)),source)
    for name in module._FORBIDDEN_RUNTIME_TYPE_PARTS:
        for annotation in [name,repr(name),'list['+name+']','dict[str, '+name+']']:
            add('annotation-'+str(len(rows)),'value: '+annotation)
    for name in sorted(module._FORBIDDEN_BUILDER_CALLS):
        for style in ['ordinary','async','nested','decorator','default','return-annotation']:
            body=dict(ordinary=f'    def f(self):\n        self.x.{name}()\n',
                **{'async':f'    async def f(self):\n        self.x.{name}()\n',
                'nested':f'    def f(self):\n        def inner():\n            self.x.{name}()\n',
                'decorator':f'    @x.{name}()\n    def f(self): pass\n',
                'default':f'    def f(self, value=x.{name}()): pass\n',
                'return-annotation':f'    def f(self) -> x.{name}(): pass\n'})[style]
            add(f'terminal-{name}-{style}','class SemanticPlanBuilder:\n'+body)
    for root in ['dict','list','set']:
        for annotation in [root,root+'[str]','typing.'+root+'[str]',repr(root),root+'[str][int]']:
            for method in ['public','_private']:
                add('return-'+str(len(rows)),f'class SemanticPlanBuilder:\n    def {method}(self) -> {annotation}: pass\n')
    for i,body in enumerate(['pass','RefRemap','rewrite_exact_plan_refs_payload',
        'RefRemap; rewrite_exact_plan_refs_payload','obj.RefRemap; obj.rewrite_exact_plan_refs_payload',
        'f(RefRemap=1, rewrite_exact_plan_refs_payload=2)',
        'def inner():\n            RefRemap; rewrite_exact_plan_refs_payload']):
        add('fragment-'+str(i),'class SemanticPlanBuilder:\n    def add_fragment(self):\n        '+body+'\n')
    for label,source in [
        ('fragment-first','class SemanticPlanBuilder:\n    def add_fragment(self): pass\n    def add_fragment(self):\n        RefRemap; rewrite_exact_plan_refs_payload\n'),
        ('fragment-async','class SemanticPlanBuilder:\n    async def add_fragment(self): pass\n'),
        ('fragment-conditional','class SemanticPlanBuilder:\n    if True:\n        def add_fragment(self): pass\n'),
        ('nested-builder','def outer():\n    class SemanticPlanBuilder:\n        def f(self): self.x.execute()\n'),
        ('conditional-builder','if True:\n    class SemanticPlanBuilder:\n        def f(self): self.x.execute()\n'),
        ('decorated-builder','@wrapper\nclass SemanticPlanBuilder:\n    def f(self): self.x.execute()\n'),
        ('nfkc-builder','class ＳemanticPlanBuilder:\n    def f(self): self.x.execute()\n'),
        ('nfkc-import','import ｔransaction\n'),('nfkc-method','class SemanticPlanBuilder:\n    def f(self): self.x.ｅxecute()\n'),
        ('imports-one-event','import transaction_cache, executor_connection'),
        ('annotation-one-event','value: TransactionConnection'),
        ('bfs-module','def f():\n    import transaction\nx: Connection\nclass X:\n    y: Cursor\n'),
        ('method-keyword-order','class SemanticPlanBuilder:\n    def f(self):\n        f(key=x.execute(), *x.commit())\n'),
        ('method-dictionary-order','class SemanticPlanBuilder:\n    def f(self):\n        {x.execute():x.commit(),x.resolve():x.rollback()}\n'),
        ('method-conditional-order','class SemanticPlanBuilder:\n    def f(self):\n        x.execute() if x.commit() else x.resolve()\n'),
        ('pass-order','class SemanticPlanBuilder:\n    x: Transaction\n    def f(self) -> list[int]:\n        x.execute()\n    def add_fragment(self):\n        x.commit()\nimport cache\n'),
        ('empty',''),('clean','class SemanticPlan: pass\n'),('nul','\x00'),('syntax','def broken(:\n')]:add(label,source)
    return rows


def exercise(module,case,temporary):
    path=temporary/'semantic_plan.py'
    def normalize(value):
        if isinstance(value,str):return value.replace(str(temporary),'$TMP')
        if isinstance(value,list):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    try:value=dict(value=canonical(module.scan_semantic_source(path,case['source'])))
    except BaseException as error:value=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(value)


if __name__=='__main__':
    from contract_runner import main
    from composition_port import install
    main(CHECK,'rerail.composition-source-contract/v1',cases,exercise,install,
         ['composition_source_contract.py','composition_port.py','contract_runner.py'])
