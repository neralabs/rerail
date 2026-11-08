"""Build recorded helper, lexical-call, Unicode-boundary and production-scope contracts."""
import argparse
import json
from pathlib import Path


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--baseline',type=Path,required=True);p.add_argument('--api',type=Path,required=True);p.add_argument('--reporters',type=Path,required=True);a=p.parse_args();api=[];report=[];baseline=json.loads(a.baseline.read_text())
    for i,call in enumerate(baseline['calls']):
        if call['entry'] not in {'_is_false_positive','_is_sql_syntax_pattern','_is_error_msg_pattern','_classify_pattern'}:continue
        arguments=dict(call['arguments']['items']);api.append(dict(id='original-helper-'+str(i),check='regex_sql_parsing',entry=call['entry'],files=[],arguments=arguments))
    patterns=['SELECT value FROM table','%s','%%s','%%%s','\\?','no such table','ON DELETE CASCADE',r'\bCHECK\s*\(',r'BEFORE INSERT.*WHEN.*BEGIN',' LIKE ',r'/\*.*?\*/',r'--[^\n]*','fts5','json_valid','mode=[^&]*','api_key','password','KEY','IN (?,?)','plain words','SELECT\nFROM']
    for i,pattern in enumerate(patterns):
        for j,decoration in enumerate(['','漢','\u0301','²','\u200c','\u00a0','İ','ı','ſ','K','\x1c']):
            value=decoration+pattern+decoration
            api.append(dict(id=f'classification-{i}-{j}',check='regex_sql_parsing',entry='_classify_pattern',files=[],arguments=dict(pattern_str=value,func_name='sub' if i%2 else 'search')))
    sources=[]
    for i,call in enumerate(baseline['calls']):
        if call['entry']=='check_file':sources.append(('original-checker-'+str(i),dict(call['arguments']['items'])['source']))
    for i,fn in enumerate(['re.search','re.sub','re.compile','re.split','re.match','re.findall','re.fullmatch','other.search','search','(re.search)','re[0].search','pkg.re.search']):
        sources.append(('call-shape-'+str(i),'result = '+fn+'('+repr(r'\bSELECT\b.*\bFROM\b')+', text)\n'))
    sources+=[('nested-calls',"import re\nx=re.search('SELECT first', re.compile('SELECT nested'))\n"),('fstring',"import re\nx=re.compile(f'SELECT {name} FROM {table}')\n"),('adjacent',"import re\nx=re.compile('SELECT ' 'value')\n"),('keyword',"import re\nx=re.compile(pattern='SELECT value')\n"),('unicode-alias',"rｅ.search('SELECT value', text)\n"),('skip-aliased',"import re as regex\nregex.search('SELECT value', text)\n"),('false-positive',"import re\nre.compile('password SELECT')\n"),('long-preview','import re\nre.compile('+repr('SELECT '+('漢🐍'*36))+')\n')]
    for identity,source in sources:
        path='impl/case.py';files=[dict(path=path,source=source)];api.append(dict(id=identity,check='regex_sql_parsing',entry='checker',files=[],arguments=dict(tree=dict(pythonAst=source,mode='exec'),path=dict(path=path),source=source)));report.append(dict(id=identity,check='regex_sql_parsing',files=files))
    for i,(pattern,method) in enumerate([(r'/\*.*?\*/','compile'),('%s','compile'),('ON DELETE CASCADE','compile'),('BEFORE INSERT.*WHEN.*BEGIN','compile'),('no such table','compile'),('SELECT value','sub'),('SELECT value','compile')]):
        report.append(dict(id='family-'+str(i),check='regex_sql_parsing',files=[dict(path='impl/family.py',source='import re\nre.'+method+'('+repr(pattern)+', text)\n')]))
    for i,path in enumerate(['impl/a.py','data_layer/a.py','_internal/a.py','l1234_layer/a.py','l१२३४_layer/a.py','api/a.py','impl/tests/a.py','impl/test/a.py','impl/test_a.py','impl/.venv/a.py','impl/tmp/a.py','impl/conform/a.py','other/a.py']):report.append(dict(id='scope-'+str(i),check='regex_sql_parsing',files=[dict(path=path,source="import re\nre.compile('SELECT value')\n")]))
    for path,cases in [(a.api,api),(a.reporters,report)]:path.write_text(json.dumps(dict(format='rerail.regex-sql-cases/v1',cases=cases),indent=2)+'\n')
    print(json.dumps(dict(api=len(api),reporters=len(report))))


if __name__=='__main__':main()
