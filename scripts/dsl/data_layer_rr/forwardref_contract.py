"""Record ForwardRef event order, state resets, source gates, and complete public DTOs."""
from classvar_contract import exercise
from forwardref_scope_contract import install
from contract_runner import main

CHECK='forwardref'
ASSIGN='annotation = field.annotation\n'
CALL='callable(annotation)\n'


def cases(_module):
    rows=[]
    calls=['isinstance(annotation, type)','issubclass(annotation, object)','callable(annotation)',
           'isinstance(annotation, (type,))','isinstance(annotation, type, ForwardRef)',
           'isinstance(annotation, other)','isinstance(other, type)','issubclass(other, object)',
           'callable(other)','typing.callable(annotation)','isinstance(annotation=annotation, class_or_tuple=type)',
           'isinstance(*annotation, type)','issubclass(annotation)','callable(annotation, other)',
           '(callable)((annotation))','f"{callable(annotation)}"']
    rows.extend(dict(id=f'call-{i}',kind='file',disk=ASSIGN+call+'\n') for i,call in enumerate(calls))
    guards=['ForwardRef','typing.ForwardRef','(str, ForwardRef)','((str,),(typing.ForwardRef,))',
            '"ForwardRef"','[ForwardRef]','Optional[ForwardRef]','str | ForwardRef','(ForwardRef and str)']
    rows.extend(dict(id=f'guard-{i}',kind='file',disk=ASSIGN+f'isinstance(annotation, {guard})\n'+CALL) for i,guard in enumerate(guards))
    assignments=['annotation = field.annotation','annotation = other = field.annotation',
                 '(annotation) = field.annotation','annotation: object = field.annotation',
                 'annotation, other = field.annotation','other.annotation = field.annotation',
                 'annotation = field.other','other = field.annotation','annotation += field.annotation',
                 'ａｎｎｏｔａｔｉｏｎ = field.ａｎｎｏｔａｔｉｏｎ',
                 '((annotation)) = field.annotation','(annotation,) = field.annotation',
                 '( # a comment, not a tuple\n annotation\n) = field.annotation',
                 '((annotation,)) = field.annotation']
    rows.extend(dict(id=f'assignment-{i}',kind='file',disk=assignment+'\n'+CALL) for i,assignment in enumerate(assignments))
    shapes={
        'clean':'pass\n','invalid':'def broken(: # ForwardRef\n',
        'duplicate':ASSIGN+CALL+CALL,
        'guard-before-assignment':'isinstance(annotation, ForwardRef)\n'+ASSIGN+CALL,
        'guard-reset':ASSIGN+'isinstance(annotation, ForwardRef)\n'+CALL+ASSIGN+CALL,
        'sync-reset':ASSIGN+'def inner():\n    '+CALL+CALL,
        'async-preserved':ASSIGN+'async def inner():\n    '+CALL+CALL,
        'sync-restored':ASSIGN+'def inner():\n    '+ASSIGN+'    isinstance(annotation, ForwardRef)\n'+CALL,
        'class-preserved':ASSIGN+'class C:\n    '+CALL+CALL,
        'lambda-preserved':ASSIGN+'consume = lambda: callable(annotation)\n',
        'comprehension':ASSIGN+'values = [callable(annotation) for x in range(2)]\n',
        'body-before-decorator':'@decorate(callable(annotation))\ndef check(value=callable(annotation)):\n    '+ASSIGN+'    '+CALL,
        'class-decorator':ASSIGN+'@decorate(callable(annotation))\nclass C:\n    isinstance(annotation, ForwardRef)\n'+CALL,
        'nested-check-order':ASSIGN+'callable(annotation, isinstance(annotation, ForwardRef))\n'+CALL,
        'reset-inside-rhs':ASSIGN+'isinstance(annotation, ForwardRef)\nannotation = field.annotation(callable(annotation))\n'+CALL,
        'unicode-column':ASSIGN+'é = 1; '+CALL,
    }
    rows.extend(dict(id='shape-'+name,kind='file',disk=source) for name,source in shapes.items())
    for path in ['impl/a.py','outside/a.py','tests/a.py','test/a.py','conform/a.py','tmp/a.py','impl/test_a.py','.py','.hidden.py','a.pyi']:
        rows.append(dict(id='path-'+path,kind='file',path=path,disk=ASSIGN+CALL))
    for kind in ['file','content']:
        rows.append(dict(id=kind+'-missing',kind=kind,disk=None,supplied=ASSIGN+CALL))
        rows.append(dict(id=kind+'-invalid-utf8',kind=kind,binary='ff',supplied=ASSIGN+CALL))
    for disk_name,disk in [('positive',ASSIGN+CALL),('clean','pass\n'),('invalid','def broken(: # ForwardRef\n')]:
        for gate_name,gate in [('positive',ASSIGN+CALL),('clean','pass\n')]:
            rows.append(dict(id=f'content-{disk_name}-{gate_name}',kind='content',disk=disk,supplied=gate))
    rows.extend([dict(id='content-unreadable-excluded',kind='content',path='tests/a.py',binary='ff',supplied=ASSIGN+CALL),
                 dict(id='content-unreadable-clean-gate',kind='content',binary='ff',supplied='pass\n'),
                 dict(id='relative',kind='file',relative=True,disk=ASSIGN+CALL),
                 dict(id='root-under-tests',kind='content',rootPath='tests/project',disk=ASSIGN+CALL,supplied=ASSIGN+CALL),
                 dict(id='directory',kind='directory',files=[dict(path=p,source=ASSIGN+CALL) for p in ['z.py','sub/a.py','tests/a.py','.py']],exclude=['/sub/'])])
    return rows


if __name__=='__main__':
    main(CHECK,'rerail.forwardref-public/v1',cases,exercise,install,
         ['forwardref_contract.py','forwardref_scope_contract.py','classvar_contract.py','suite.py'])
