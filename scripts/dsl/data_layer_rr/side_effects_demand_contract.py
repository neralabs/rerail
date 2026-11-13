"""Source short-circuiting must happen before expensive supplied-value capture."""
from pathlib import Path
from types import SimpleNamespace
import sys
from contract_runner import main
from dto import canonical
from side_effects_plugin_port import install as configure

CHECK='plan_scope_side_effects'


def cases(_module):return [dict(kind='demand',id=name)for name in ['inactive-row','unused-field','whole-positive-batch']]


def exercise(module,case,scratch):
    saved=sys.get_int_max_str_digits();sys.set_int_max_str_digits(4300)
    try:
        root=scratch/'repo';root.mkdir()
        row=dict(code='PSE-020',file_path='supplied.py',line_number=3,message='selected',severity='error')
        if case['id']=='inactive-row':row.update(severity='audit',message=10**5000)
        else:row['unused']=10**5000
        family=module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id
        rows=[dict(row,line_number=i)for i in range(100)]if case['id']=='whole-positive-batch'else [row]
        bundle=SimpleNamespace(path=Path('bundle.py'),facts={family:rows})
        plugin=module._plan_scope_side_effects_plugin(root,module.FileScopeSpec())
        return dict(value=canonical(plugin.reporter(SimpleNamespace(root=root,files={'caller':bundle})),scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    finally:sys.set_int_max_str_digits(saved)


def install(backend,module,detectors=False):configure(backend,module)


if __name__=='__main__':main(CHECK,'rerail.side-effects-demand/v1',cases,exercise,install,['side_effects_demand_contract.py','side_effects_plugin_port.py','value_columns.py'])
