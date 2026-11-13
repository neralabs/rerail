"""Unread mapping values must not prevent conversion of a valid source fact."""
from contract_runner import main
from dto import canonical


def cases(_module):
    return [dict(kind='unused',id=name)for name in ['integer','nested-integer','integer-key']]


def exercise(module,case,scratch):
    raw=dict(code='C',file_path='a.py',line_number=3,message='message')
    unread=10**5000
    if case['id']=='integer':raw['unused']=unread
    elif case['id']=='nested-integer':raw['unused']={'nested':[unread]}
    else:raw[unread]='unused'
    try:return dict(value=canonical(module._violation_from_fact(raw),scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from side_effects_conversion_port import install as conversions
    conversions(backend,module)


if __name__=='__main__':main('plan_scope_side_effects','rerail.side-effects-conversion-demand/v1',cases,exercise,install,['side_effects_conversion_demand_contract.py','side_effects_conversion_port.py','value_columns.py'])
