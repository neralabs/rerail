"""Generate independent render-context, interpolation, preview, filesystem, and helper contracts."""
import argparse
import json
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api',type=Path,required=True);parser.add_argument('--reporters',type=Path,required=True)
    args=parser.parse_args();api=[];reporters=[];filename='impl/common/l2100_sql_grammar/example.py'
    sources=[
        ('plain','def render():\n return f"SELECT {value}"\n'),
        ('async','class SQL:\n async def render_select(self):\n  return f"{self.value}"\n'),
        ('nonrender','def renderable():\n return f"{value}"\n'),
        ('render-prefix','def render_():\n return f"{value}"\n'),
        ('nested-functions','def render_outer():\n def render_inner():\n  return f"inner {value}"\n def other():\n  return f"other {value}"\n return f"outer {value}"\n'),
        ('nested-classes','class Outer:\n def render_a(self):\n  class Inner:\n   def render_b(self):\n    return f"inner {value}"\n  return f"outer {value}"\n'),
        ('siblings','class A:\n def render(self):return f"a{x}"\nclass B:\n def render_other(self):return f"b{x}"\n'),
        ('decorators','class A:\n @deco(f"deco{x}")\n def render(self,x=f"arg{v}")->f"ret{t}":return f"body{v}"\n'),
        ('class-decorators','@deco(f"outside{x}")\nclass A:\n @deco(f"inside{x}")\n def render(self):return f"body{x}"\n'),
        ('lambda','def render():return lambda:f"lambda {x}"\n'),
        ('class-base','def render():\n class C(base(f"base{x}")):\n  value=f"body{x}"\n'),
        ('conditional','if enabled:\n def render():\n  if value:return f"yes{x}"\n  return f"no{x}"\n'),
        ('format-spec','def render():return f"{value:>10}"\n'),
        ('empty-format','def render():return f"{value:}"\n'),
        ('dynamic-format','def render():return f"{value:>{width}.{precision}f}"\n'),
        ('nested-fstring','def render():return f"{f\'inner {x}\':>{width}}tail"\n'),
        ('adjacent','def render():return "plain" f"a{x}" f"b{y}" "tail"\n'),
        ('adjacent-no-field','def render():return f"a" "b"\n'),
        ('empty','def render():return f""\n'),
        ('debug','def render():return f"{ x = }{x=!r}{x=:>8}"\n'),
        ('escaped','def render():return f"left\\n{{braces}} {x} \\N{SNOWMAN}"\n'),
        ('raw','def render():return rf"left\\n{{braces}}{x}right"\n'),
        ('multiline','def render():return f"""top\n{x:\n>{w}}tail"""\n'),
        ('long-ascii','def render():return f"'+('x'*51)+'{value}"\n'),
        ('boundary-50','def render():return f"'+('x'*45)+'{value}"\n'),
        ('long-unicode','def render():return f"'+('漢🐍'*27)+'{value}"\n'),
        ('nul','def render():return f"before\\x00'+('x'*55)+'{value}"\n'),
        ('source-triggers','def ｒｅｎｄｅｒ():return F"{x}"\n'),
        ('canonical-name','class Ａ:\n def ｒｅｎｄｅｒ(self):return f"{x}"\n'),
        ('clean','class A:\n def render(self):return value.render()\n'),
        ('outside','value=f"outside{x}"\n'),
        ('syntax-error','def render(:\n'),
    ]
    for identity,source in sources:
        files=[dict(path=filename,source=source)]
        api.append(dict(id=identity+'-file',check='render_fstrings',entry='analyze_file',files=files,arguments=dict(filepath=dict(path=filename))))
        api.append(dict(id=identity+'-source',check='render_fstrings',entry='analyze_source',files=files,arguments=dict(filepath=dict(path=filename),source='render supplied, ignored',tree=dict(pythonAst='pass\n',mode='exec'))))
        if identity!='syntax-error':
            for stats in [False,True]:
                api.append(dict(id=identity+('-stats' if stats else '-collector'),check='render_fstrings',entry='collector',files=[],arguments=dict(filepath=dict(path=filename),root=dict(path='.'),source='supplied source ignored',tree=dict(pythonAst=source,mode='exec'),stats=stats)))
        reporters.append(dict(id=identity,check='render_fstrings',files=files))
    positive='class A:\n def render(self):return f"SELECT {x:>{w}}"\n'
    for i,path in enumerate([filename,'impl/common/l2100_sql_grammar/tests/a.py','impl/common/l2100_sql_grammar/test_a.py','impl/common/l2100_sql_grammar/sub/a.py','impl/common/l2100_sql_grammar/tmp/a.py','impl/common/l2100_sql_grammar/.venv/a.py','impl/common/l2100_sql_grammar/__pycache__/a.py','impl/common/l2100_sql_grammar/conform/a.py','impl/other/a.py','data_layer/a.py','impl/common/l2100_sql_grammar/a.txt']):
        files=[dict(path=path,source=positive)]
        reporters.append(dict(id='scope-'+str(i),check='render_fstrings',files=files))
        api.append(dict(id='scope-'+str(i),check='render_fstrings',entry='collect_reports',files=files,arguments=dict(root=dict(path='.'))))
    for i,name in enumerate(['render','render_','render_sql','renderable','_render','Render','render\n','render_漢字','ｒｅｎｄｅｒ','']):
        api.append(dict(id='name-'+str(i),check='render_fstrings',entry='_is_render_method',files=[],arguments=dict(name=name)))
    for i,given in enumerate(['','pass\n','# render\n']):
        api.append(dict(id='source-gate-'+str(i),check='render_fstrings',entry='analyze_source',files=[dict(path=filename,source=positive)],arguments=dict(filepath=dict(path=filename),source=given)))
    api.append(dict(id='missing-file',check='render_fstrings',entry='analyze_file',files=[],arguments=dict(filepath=dict(path=filename))))
    api.append(dict(id='decode-file',check='render_fstrings',entry='analyze_file',files=[dict(path=filename,bytesHex='ff')],arguments=dict(filepath=dict(path=filename))))
    multi=[dict(path='impl/common/l2100_sql_grammar/z.py',source=positive),dict(path='impl/common/l2100_sql_grammar/a.py',source='def render():return f"a{x}" f"b{y}"\n'),dict(path='impl/common/l2100_sql_grammar/empty.py',source='x=1\n')]
    for i,targets in enumerate([None,[],['impl'],['impl/common/l2100_sql_grammar','impl/common/l2100_sql_grammar'],['missing']]):
        api.append(dict(id='dirs-'+str(i),check='render_fstrings',entry='collect_reports',files=multi,arguments=dict(root=dict(path='.'),target_dirs=targets)))
    for i,files in enumerate([[],multi,[dict(path=filename,source='def render():return 1\n')],[dict(path=filename,source=positive)]]):
        api.append(dict(id='formats-'+str(i),check='render_fstrings',entry='formats',files=files,arguments=dict(root=dict(path='.'))))
    for mode in ['summary','detailed']:
        for i,violations in enumerate([[],[dict(file=dict(path='outside.py',relative=True),method_name='render',class_name=None,line_number=3,fstring_preview='f"{...}"')]]):
            api.append(dict(id='outside-'+mode+'-'+str(i),check='render_fstrings',entry='format_supplied',files=[],arguments=dict(root=dict(path='sub',relative=True),mode=mode,reports=[dict(path=dict(path='outside.py',relative=True),methods=1,violations=violations)])))
    reporters.append(dict(id='multi-file',check='render_fstrings',files=multi))
    for path,cases in [(args.api,api),(args.reporters,reporters)]:path.write_text(json.dumps(dict(format='rerail.render-fstrings-contract-cases/v1',cases=cases),indent=2)+'\n')
    print(json.dumps(dict(api=len(api),reporters=len(reporters))))


if __name__=='__main__':main()
