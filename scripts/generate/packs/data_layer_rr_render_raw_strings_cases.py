"""Generate independent raw-render collector, exemption, preview, and formatting contracts."""
import argparse
import json
from pathlib import Path


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--api',type=Path,required=True);p.add_argument('--reporters',type=Path,required=True);args=p.parse_args()
    api=[];reporters=[];filename='impl/common/l2100_sql_grammar/example.py'
    def add(identity,source,graph=True):
        files=[dict(path=filename,source=source)]
        api.append(dict(id=identity+'-file',check='render_raw_strings',entry='analyze_file',files=files,arguments=dict(filepath=dict(path=filename))))
        api.append(dict(id=identity+'-source',check='render_raw_strings',entry='analyze_source',files=files,arguments=dict(filepath=dict(path=filename),source='render supplied, ignored',tree=dict(pythonAst='pass\n',mode='exec'))))
        if graph:
            for stats in [False,True]:api.append(dict(id=identity+('-stats' if stats else '-collector'),check='render_raw_strings',entry='collector',files=[],arguments=dict(filepath=dict(path=filename),root=dict(path='.'),source='provided text ignored',tree=dict(pythonAst=source,mode='exec'),stats=stats)))
        reporters.append(dict(id=identity,check='render_raw_strings',files=files))
    snippets=[
        ('literal',"class A:\n def render(self):return 'SELECT value'\n"),
        ('fragment',"def render():\n prefix=' WHERE '\n return prefix\n"),
        ('docstring',"def render():\n 'function documentation'\n return 'SELECT value'\n"),
        ('nested-docstrings',"def render():\n 'outer documentation'\n def inner():\n  'inner documentation'\n  return 'SELECT inner'\n class Nested:\n  'class documentation'\n  def render_other(self):return 'SELECT method'\n return 'SELECT outer'\n"),
        ('nested-render',"class A:\n def render(self):\n  def render_b():return 'SELECT nested'\n  return 'SELECT outer'\n"),
        ('siblings',"class A:\n def render(self):return 'SELECT A'\nclass B:\n async def render_b(self):return 'SELECT B'\n"),
        ('decorators',"@deco('SELECT deco')\ndef render(x='SELECT arg')->'SELECT annotation':return 'SELECT body'\n"),
        ('docstring-parens',"def render():\n (\n # comment\n 'doc' 'string'\n )\n return 'SELECT value'\n"),
        ('async',"async def render_sql():return 'SELECT async'\n"),
        ('prefix',"def render_():return 'SELECT prefix'\n"),
        ('nonrender',"def renderable():return 'SELECT outside'\n"),
        ('nested-raise',"def render():raise Error('SELECT raise', lambda:'SELECT lambda')\n"),
        ('assert',"def render():\n assert x=='SELECT assert', 'SELECT message'\n"),
        ('special-raise',"def render():raise Error(Template('SELECT raise'), SqlFragment.static('SELECT static'))\n"),
        ('concat',"def render():return 'SELECT ' + value + ' WHERE '\n"),
        ('adjacent',"def render():return ('SELECT ' # comment\n 'value')\n"),
        ('lambda',"def render():return lambda:'SELECT lambda'\n"),
        ('bytes',"def render():return b'SELECT bytes'\n"),
        ('empty',"def render():return ''\n"),
        ('one',"def render():return 'x'\n"),
        ('nul',"def render():return '\\0SELECT value'\n"),
        ('repr-quotes',"def render():return \"it's a \\\"value\\\"\"\n"),
        ('repr-named',"def render():return '\\N{SNOWMAN}\\t\\u2028'\n"),
        ('long-ascii',"def render():return '"+('x'*65)+"'\n"),
        ('long-unicode',"def render():return '"+('漢🐍'*30)+"'\n"),
        ('pure-fstring',"def render():return f'SELECT literal'\n"),
        ('fstring',"def render():return f'SELECT {value} WHERE x'\n"),
        ('fstring-format',"def render():return f'SELECT {value:width>{width}}tail'\n"),
        ('fstring-debug',"def render():return f'{ value = }tail'\n"),
        ('fstring-nested',"def render():return f'prefix {Template(\"SELECT nested\")} suffix'\n"),
        ('fstring-call-context',"def render():return repr(f'SELECT {value} suffix')\n"),
        ('fstring-empty-adjacent',"def render():return ('' f'SELECT {value}' ' tail')\n"),
        ('fstring-multi-spec',"def render():return f'{value:first>10}{other:second>10}'\n"),
        ('fstring-nested-string',"def render():return f'prefix {\"SELECT inner\"} suffix'\n"),
        ('clean',"def render():return self.value.render()\n"),
        ('invalid',"def render(:\n"),
        ('unicode-name',"class Ａ:\n def ｒｅｎｄｅｒ(self):return 'SELECT canonical'\n"),
    ]
    for identity,source in snippets:add(identity,source,identity!='invalid')
    for i,callee in enumerate(['Template','pkg.Template','(Template)','SqlFragment.static','(SqlFragment).static','pkg.SqlFragment.static','SF.static','SqlFragment.other','Template[0]']):
        for j,value in enumerate(["'SELECT value'","''","'x'","b'SELECT bytes'","f'SELECT {value}'","1","('SELECT ' 'value')"]):
            add('call-'+str(i)+'-'+str(j),'def render():return '+callee+'('+value+')\n')
    for i,call in enumerate(["Template(value='SELECT keyword')","Template(name, 'SELECT second')","Template(*['SELECT star'])","SqlFragment.static('SELECT first', 'SELECT second')","Template(('SELECT nested',))","Template(**{'key':'SELECT mapping'})","Template(1, 'SELECT second')"]):add('call-shape-'+str(i),'def render():return '+call+'\n')
    allowed=['format','str','repr','SqlAliasName','SqlTableName','SqlColumnName','SqlIndexName','SqlQuotedIdentifier','SqlQualifiedIdentifier','SqlIntegerLiteral','append','join','replace','startswith','endswith','param','unknown']
    for i,name in enumerate(allowed):
        for j,call in enumerate([name+"('SELECT value')",'obj.'+name+"('SELECT value')",name+"(key='SELECT keyword')"]):add('allowed-'+str(i)+'-'+str(j),'def render():return '+call+'\n')
    positive="class A:\n def render(self):\n  x=Template('SELECT template')\n  y=SqlFragment.static('WHERE static')\n  return 'SELECT literal'\n"
    for i,path in enumerate([filename,'impl/common/l2100_sql_grammar/tests/a.py','impl/common/l2100_sql_grammar/test_a.py','impl/common/l2100_sql_grammar/tmp/a.py','impl/common/l2100_sql_grammar/.venv/a.py','impl/common/l2100_sql_grammar/__pycache__/a.py','impl/common/l2100_sql_grammar/conform/a.py','impl/other/a.py','data_layer/a.py','impl/common/l2100_sql_grammar/a.txt']):
        files=[dict(path=path,source=positive)];reporters.append(dict(id='scope-'+str(i),check='render_raw_strings',files=files));api.append(dict(id='scope-'+str(i),check='render_raw_strings',entry='collect_reports',files=files,arguments=dict(root=dict(path='.'))))
    for i,name in enumerate(['render','render_','render_sql','renderable','_render','Render','render\n','render_漢字','ｒｅｎｄｅｒ','']):api.append(dict(id='name-'+str(i),check='render_raw_strings',entry='_is_render_method',files=[],arguments=dict(name=name)))
    for i,given in enumerate(['','pass\n','# render\n']):api.append(dict(id='source-gate-'+str(i),check='render_raw_strings',entry='analyze_source',files=[dict(path=filename,source=positive)],arguments=dict(filepath=dict(path=filename),source=given)))
    api.append(dict(id='missing-file',check='render_raw_strings',entry='analyze_file',files=[],arguments=dict(filepath=dict(path=filename))))
    api.append(dict(id='decode-file',check='render_raw_strings',entry='analyze_file',files=[dict(path=filename,bytesHex='ff')],arguments=dict(filepath=dict(path=filename))))
    multi=[dict(path='impl/common/l2100_sql_grammar/z.py',source=positive),dict(path='impl/common/l2100_sql_grammar/a.py',source="def render():return 'SELECT a'\n"),dict(path='impl/common/l2100_sql_grammar/empty.py',source='x=1\n')]
    for i,targets in enumerate([None,[],['impl'],['impl/common/l2100_sql_grammar','impl/common/l2100_sql_grammar'],['missing']]):api.append(dict(id='dirs-'+str(i),check='render_raw_strings',entry='collect_reports',files=multi,arguments=dict(root=dict(path='.'),target_dirs=targets)))
    for i,files in enumerate([[],multi,[dict(path=filename,source='def render():return 1\n')],[dict(path=filename,source=positive)]]):api.append(dict(id='formats-'+str(i),check='render_raw_strings',entry='formats',files=files,arguments=dict(root=dict(path='.'))))
    for mode in ['summary','detailed']:
        for i,violations in enumerate([[],[dict(file=dict(path='outside.py',relative=True),method_name='render',class_name=None,line_number=3,violation_type='string_literal',preview="'SELECT value'")]]):api.append(dict(id='outside-'+mode+'-'+str(i),check='render_raw_strings',entry='format_supplied',files=[],arguments=dict(root=dict(path='sub',relative=True),mode=mode,reports=[dict(path=dict(path='outside.py',relative=True),methods=1,violations=violations)])))
    api.append(dict(id='autofix-hints',check='render_raw_strings',entry='_autofix_hint_lines',files=[],arguments={}))
    reporters.append(dict(id='multi-file',check='render_raw_strings',files=multi))
    for path,cases in [(args.api,api),(args.reporters,reporters)]:path.write_text(json.dumps(dict(format='rerail.render-raw-strings-contract-cases/v1',cases=cases),indent=2)+'\n')
    print(json.dumps(dict(api=len(api),reporters=len(reporters))))


if __name__=='__main__':main()
