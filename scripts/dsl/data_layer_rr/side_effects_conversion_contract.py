"""Record complete conversion helpers before replacing their Python implementations."""
import ast
from pathlib import Path
from contract_runner import main
from dto import canonical

CHECK='plan_scope_side_effects'


def cases(_module):
    rows=[]
    for i,severity in enumerate(['error','audit','warning',None,True]):
        rows.append(dict(kind='to_fact',id='to-fact-'+str(i),code='C',path='é\nname.py',line=[1,-2,True,10**80,None][i],message='message\nnext',severity=severity))
    paths=[('project','project/a.py'),('project','project'),('project','other/a.py'),('/root','/root/a.py'),
        ('/root','/rooted/a.py'),('/','/a.py'),('//','//a.py'),('.','a.py'),('project','project/a/../b.py')]
    for i,(root,path)in enumerate(paths):
        for j,severity in enumerate(['error','audit','hard-fail',None,True]):
            rows.append(dict(kind='structured',id=f'structured-{i}-{j}',root=root,path=path,severity=severity,line=7))
    for i,line in enumerate([True,10**80,None]):
        rows.append(dict(kind='structured',id='structured-line-'+str(i),root='project',path='project/a.py',severity='audit',line=line))
    lines=[0,-2,True,10**80,'002','-004','+3',' \t3\n','1_234','٠٣','１２','٠_۳','0x10','1.2','1__2','',None,[],{},3.8,-3.8,-0.0,1e20,
        '²','½','＋3','−3','1٬234','\x1c3','\u20033\u00a0','1'*4300,'1'*4301,'0'*5001]
    base=dict(code='C',file_path='a.py',line_number=3,message='message')
    for i,line in enumerate(lines):rows.append(dict(kind='fact',id='fact-line-'+str(i),raw=dict(base,line_number=line)))
    for name in ['nan','inf','-inf']:
        rows.append(dict(kind='fact',id='fact-'+name,raw=base,line_special=name))
    for key,value in [('code',None),('code',True),('file_path',123),('message',[True,None,1.25]),
            ('severity','audit'),('severity',None),('severity',True),('severity',42),('severity',{'x':[1,None]})]:
        rows.append(dict(kind='fact',id='fact-field-'+str(len(rows)),raw=dict(base,**{key:value})))
    for key in base:rows.append(dict(kind='fact',id='fact-missing-'+key,raw={k:v for k,v in base.items()if k!=key}))
    for i,source in enumerate(['','pass\n','with open_write_plan(): lazy.resolve()\n']):
        rows.append(dict(kind='extract_stub',id='extract-stub-'+str(i),source=source,tree=i!=0))
    return rows


def exercise(module,case,scratch):
    try:
        if case['kind']=='to_fact':
            value=module._violation_to_fact(module.Violation(case['code'],case['path'],case['line'],case['message'],case['severity']))
        elif case['kind']=='structured':
            finding=module.StructuredViolation('caller','C',Path(case['path']),case['line'],'message',case['severity'])
            value=module._violation_from_structured(Path(case['root']),finding)
        elif case['kind']=='fact':
            raw=dict(case['raw'])
            if 'line_special'in case:raw['line_number']=float(case['line_special'])
            value=module._violation_from_fact(raw)
        else:
            value=module._extract_plan_scope_side_effect_facts(Path('caller-root'),Path('elsewhere.py'),case['source'],ast.parse('pass\n')if case['tree']else None)
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(_backend,_module,detectors=False):
    # The source-call guard deliberately exposes these remaining unported APIs.
    pass


if __name__=='__main__':main(CHECK,'rerail.side-effects-conversion/v1',cases,exercise,install,['side_effects_conversion_contract.py'])
