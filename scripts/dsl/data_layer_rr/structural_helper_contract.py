"""Record the complete shared connection helper with supplied trees and positive witnesses."""
import ast
import contextlib
from dataclasses import fields
from pathlib import Path
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from fresh import invoke

CHECK='connection_structural_boundaries'


def cases(module):
    rows=[dict(id='metadata',kind='metadata')]
    for name in sorted(module.CONNECTION_RESIDUAL_SYMBOLS):
        for kind in ['class','def','async def']:
            source=f'{kind} {name}'+(':\n pass\n' if kind=='class' else '():\n pass\n')
            rows.append(dict(id=kind+'-'+name,kind='helper',tree=source,source='not valid Python !!!\n'))
    shapes={
        'clean':'pass\n',
        'nested':'class Container:\n class WritePlanWorkflow: pass\n',
        'allowed':'class _DatabaseConnectionCore(_ConnectionStateCore, _LazyConnectionBase, ConnectionPayloadOpaqueMarker): pass\n',
        'forbidden':'class _DatabaseConnectionCore(ZMixin, AMixin): pass\n',
        'attribute':'class _DatabaseConnectionCore(mod.ZMixin, mod.AMixin): pass\n',
        'duplicates':'class _DatabaseConnectionCore(ZMixin, AMixin, ZMixin): pass\n',
        'ignored-bases':'class _DatabaseConnectionCore(Generic[T], make_base()): pass\n',
        'first-core-wins':'class _DatabaseConnectionCore(_ConnectionStateCore): pass\nclass _DatabaseConnectionCore(Bad): pass\n',
        'first-core-bad':'class _DatabaseConnectionCore(Bad): pass\nclass _DatabaseConnectionCore(_ConnectionStateCore): pass\n',
        'function-before-core':'def _DatabaseConnectionCore(): pass\nclass _DatabaseConnectionCore(Bad): pass\n',
        'nested-core':'class Container:\n class _DatabaseConnectionCore(Bad): pass\n',
        'report-order':'class WritePlanWorkflow: pass\ndef ReadPlanWorkflow(): pass\nclass _DatabaseConnectionCore(Bad): pass\n',
        'unicode-name':'class ＷｒｉｔｅＰｌａｎＷｏｒｋｆｌｏｗ: pass\n',
        'different-source':'class WritePlanWorkflow: pass\n',
    }
    rows.extend(dict(id=name,kind='helper',tree=tree,source='pass\n') for name,tree in shapes.items())
    for name,source,limit in [('at-limit','x\n'*3,3),('above-limit','x\n'*4,3),('empty','',0),('no-final-newline','x',0),
        ('crlf','a\r\nb\r\n',1),('carriage-return','a\rb\r',1),('unicode-lines','a\x85b\u2028c\x1cd',3),
        ('default-limit','x\n'*900,900),('default-over','x\n'*901,900)]:
        rows.append(dict(id='lines-'+name,kind='helper',tree='',source=source,limit=limit))
    for i,separator in enumerate(['\n','\r','\r\n','\v','\f','\x1c','\x1d','\x1e','\x85','\u2028','\u2029','\x1f']):
        for name,source in [('middle','a'+separator+'b'),('terminal','a'+separator+'b'+separator),('empty-lines',separator*2)]:
            rows.append(dict(id=f'separator-{i}-{name}',kind='helper',tree='',source=source,limit=1))
    rows.extend([
        dict(id='combined',kind='helper',tree=shapes['report-order'],source='a\nb\n',limit=1),
        dict(id='missing-tree',kind='helper',tree='',source='pass\n',missingTree=True),
        dict(id='missing-file',kind='helper',tree='',missingFile=True),
        dict(id='directory-read',kind='helper',tree='',directory=True),
        dict(id='invalid-utf8',kind='helper',tree='',binary='ff'),
        dict(id='non-python-name',kind='helper',tree=shapes['forbidden'],source='bad syntax',path='input.txt'),
    ])
    return rows


def exercise(module,case,scratch):
    root=scratch.resolve()/'repo';root.mkdir()
    if case['kind']=='metadata':
        return canonical(dict(constants={n:getattr(module,n) for n in ['CONNECTION_FILE_MAX_LINES','CONNECTION_RESIDUAL_SYMBOLS','ALLOWED_DATABASE_CORE_BASES']},
            dto=[dict(name=f.name,type=str(f.type)) for f in fields(module.StructuralViolation)]),scratch.resolve())
    path=root/case.get('path','input.py')
    if case.get('directory'):path.mkdir()
    elif 'binary' in case:path.write_bytes(bytes.fromhex(case['binary']))
    elif not case.get('missingFile'):path.write_bytes(case.get('source','pass\n').encode())
    trees={} if case.get('missingTree') else {path:ast.parse(case['tree'])}
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'CONNECTION_FILE',path))
        if 'limit' in case:stack.enter_context(patch.object(module,'CONNECTION_FILE_MAX_LINES',case['limit']))
        try:return invoke(lambda:module._connection_file_violations(trees),scratch.resolve())
        except AssertionError as error:return dict(unexpected=dict(type=type(error).__name__,args=canonical(error.args,scratch.resolve())))


def install(backend,module,detectors=False):
    from structural_helper_port import install as install_native
    install_native(backend,module)


if __name__=='__main__':main(CHECK,'rerail.structural-connection-helper/v1',cases,exercise,install,['structural_helper_contract.py','fresh.py'])
