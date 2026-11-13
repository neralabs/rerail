"""Exercise shared primitive formatting through the real supplied-fact reporter."""
from pathlib import Path
from types import SimpleNamespace
import struct
from contract_runner import main
from dto import canonical
from side_effects_plugin_port import install as configure

CHECK='plan_scope_side_effects'


def cases(_module):
    bits=['0000000000000000','8000000000000000','0000000000000001','8000000000000001','0010000000000000',
          '7fefffffffffffff','ffefffffffffffff','7ff0000000000000','fff0000000000000','7ff8000000000123']
    seed=0x123456789abcdef
    for _ in range(64):
        seed=(seed*6364136223846793005+1442695040888963407)&((1<<64)-1);bits.append(f'{seed:016x}')
    return [dict(kind='values',id='float-bits',bits=bits),dict(kind='values',id='nested-keys'),dict(kind='values',id='unrelated-family'),
        *[dict(kind='values',id='root-'+str(i),root=root)for i,root in enumerate(['/','//','.','relative-root'])]]


def exercise(module,case,scratch):
    try:
        physical=scratch/'repo';physical.mkdir();root=Path(case.get('root',physical.as_posix()))
        if case['id']=='float-bits':value=[struct.unpack('>d',bytes.fromhex(bits))[0]for bits in case['bits']]
        else:value={1:'integer',None:'none',(1,'x'):[True,False,(),('é',)],1.5:{'\n':'"quote\''}}
        family=module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id
        hit=dict(code='PSE-020',file_path='supplied.py',line_number=3,message=value)
        facts={family:[hit]}
        if case['id']=='unrelated-family':
            class Explosive(dict):
                def items(self):raise AssertionError('Unrequested fact family was traversed')
            facts['foreign']=Explosive(x=object())
        bundle=SimpleNamespace(path=Path('bundle.py'),facts=facts)
        plugin=module._plan_scope_side_effects_plugin(root,module.FileScopeSpec())
        return dict(value=canonical(plugin.reporter(SimpleNamespace(root=root,files={'caller':bundle})),scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):configure(backend,module)


if __name__=='__main__':main(CHECK,'rerail.side-effects-values/v1',cases,exercise,install,['side_effects_value_contract.py','side_effects_plugin_port.py','value_columns.py'])
