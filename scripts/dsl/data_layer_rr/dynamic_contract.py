"""Record dynamic-loading scanner, state-machine, helper, and metadata contracts."""
from pathlib import Path
from dto import canonical

CHECK='dynamic_module_loading'


def settings(module):
    result=dict(linter=[module.LINTER_NAME],signature=[module.ALLOWLIST_SIGNATURE],allowed=list(module.ALLOWLISTED_FILES),fast=list(module._FAST_SUBSTRINGS))
    for name,patterns in [('primary',module._PATTERNS),('imports',module._IMPORT_MODULE_PATTERNS)]:
        result[name+'.pattern']=[p.pattern for p,_ in patterns];result[name+'.flags']=[str(p.flags) for p,_ in patterns];result[name+'.label']=[label for _,label in patterns]
    return result


def cases(module):
    rows=[]
    def add(name,content,**extra):rows.append(dict(kind='content',id=name,content=content,**extra))
    for name,text in [('getattr','def __getattr__(name): pass'),('globals','globals()["x"] = 1'),('import','__import__("os")'),
        ('qualified','importlib.import_module("os")'),('bare','import_module("os")'),('combined-primary','def __getattr__(x): return globals()["x"] = __import__("x")'),
        ('primary-before-imports','def __getattr__(x): return importlib.import_module(x)'),('same-pattern-once','__import__("x") + __import__("y")'),
        ('safe','importlib.util.find_spec("os")'),('comment','# import_module("x")'),('inline-comment','pass # import_module("x")'),
        ('indented','  def __getattr__(name): pass'),('empty',''),('invalid-python','( : __import__("x")'),('unicode','é = "😀"; __import__("x")'),
        ('long','import_module("'+('x'*1200)+'")'),('trigger-spacing','import_module ("x")'),
        ('fast-elsewhere','import_module ("x")\n# import_module('),('word-boundary','éimport_module("x")'),
        ('nonword-boundary','\u0301import_module("x")')]:add(name,text)
    for delimiter in ['"""',"'''"]:
        for name,text in [('single',delimiter+'import_module("x")'+delimiter),
            ('multi',delimiter+'\nimport_module("x")\n'+delimiter+'\nimport_module("y")'),
            ('unclosed',delimiter+'\nimport_module("x")'),('closing-code',delimiter+'\ntext\n'+delimiter+'; import_module("x")'),
            ('comment-opens','# '+delimiter+'\nimport_module("x")\n'+delimiter+'\nimport_module("y")'),
            ('other-quotes',delimiter+'\n'+("'''" if delimiter=='"""' else '"""')+'\nimport_module("x")\n'+delimiter)]:add('quotes-'+delimiter+'-'+name,text)
    for space in [' ','\t','\u2000','\x1c','\u00a0']:
        for name,body in [('guard','if TYPE_CHECKING:\n'+space+'import_module("x")'),
            ('intervening','if TYPE_CHECKING:\n'+space+'pass\n'+space+'import_module("x")'),
            ('reset','if TYPE_CHECKING:\n'+space+'pass\nx=1\n'+space+'import_module("x")'),
            ('comment','if TYPE_CHECKING:\n# note\n'+space+'import_module("x")')]:add('indent-'+repr(space)+'-'+name,body)
    for separator in ['\n','\r','\r\n','\v','\f','\x1c','\x1d','\x1e','\x85','\u2028','\u2029']:
        add('separator-'+repr(separator),'pass'+separator+'import_module("x")'+separator)
    for path in ['api/a.py','impl/a.py','tests/a.py','conform/a.py','other/a.py','notes.txt','a.PY','../outside.py']:
        add('path-'+path,'import_module("x")',path=path)
    for allowed in [[],['api/a.py'],['other.py'],['api/a.py','api/a.py']]:add('allowed-'+repr(allowed),'import_module("x")',allowed=allowed)
    for name,value in [('present','__import__("x")\n'),('empty',''),('missing',None),('invalid-utf8','ff'),('directory',None)]:
        rows.append(dict(kind='file',id='file-'+name,content=value))
    for lines in [[],['x'],['if TYPE_CHECKING:',' import_module("x")'],['if TYPE_CHECKING:','','# note',' x'],
        ['if TYPE_CHECKING:',' x','z=1',' x'],['if TYPE_CHECKING:\n',' x\n'],['if TYPE_CHECKING:\nx=1',' x']]:
        for index in [-len(lines)-1,-1,0,len(lines)-1,len(lines)]:rows.append(dict(kind='predicate',id='predicate-'+str(len(rows)),lines=lines,index=index))
    rows.append(dict(kind='configuration',id='configuration'));return rows


def exercise(module,case,temporary):
    root=temporary/'root';root.mkdir();path=root/case.get('path','api/a.py')
    try:
        if case['kind']=='configuration':value=module._rerail_dynamic_configuration() if hasattr(module,'_rerail_dynamic_configuration') else settings(module)
        elif case['kind']=='predicate':value=module._is_in_type_checking_block(case['lines'],case['index'])
        else:
            path.parent.mkdir(parents=True,exist_ok=True)
            if case['kind']=='file':
                if case['id']=='file-directory':path.mkdir()
                elif case['content'] is not None:
                    if case['id']=='file-invalid-utf8':path.write_bytes(bytes.fromhex(case['content']))
                    else:path.write_text(case['content'])
                value=module._scan_file(path,root,frozenset(case.get('allowed',[])))
            else:value=module._scan_file_content(path,root,frozenset(case.get('allowed',[])),case['content'])
        return dict(value=canonical(value,temporary))
    except BaseException as error:
        return dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,temporary)))


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-source-contract/v1',cases,exercise,install,['dynamic_contract.py','dynamic_port.py'])
