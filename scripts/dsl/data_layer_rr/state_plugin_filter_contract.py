"""Exercise every signed Python-work key through the native plugin reporter."""
from contract_runner import main
from state_plugin_contract import exercise,install

CHECK='serverplan_state_tripwires'


def cases(module):
    generated={name:sorted(kinds) for name,kinds in module._load_generated_return_kinds().items()}
    keys=sorted(module.SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST |
                module.SERVERPLAN_PREPROCESS_RUNTIME_VALUE_ALLOWLIST |
                module.SERVERPLAN_RUNTIME_CALLBACK_ALLOWLIST)
    rows=[]
    for i,key in enumerate(keys):
        code,qualname,path=key.split('::',2)
        rows.append(dict(code=code,file=path,line=i+1,qualname=qualname,message='signed finding '+str(i)))
    outsider=dict(code='SPT-003',file='impl/example.py',line=9,qualname='New.compile',message='new finding')
    return [
        dict(id='all-signed-keys',kind='signed-filter',generated=generated,rows=rows),
        dict(id='signed-keys-with-new-positive',kind='signed-filter',generated=generated,rows=[*rows,outsider]),
        dict(id='signed-filter-precedes-ratchet',kind='signed-filter',generated=generated,rows=rows,allowlist='\n'.join(keys)+'\n'),
    ]


if __name__=='__main__':
    main(CHECK,'rerail.state-plugin-signed-filter-contract/v1',cases,exercise,install,
         ['state_plugin_filter_contract.py','state_plugin_contract.py','state_plugin_port.py','state_tripwires_port.py'])
