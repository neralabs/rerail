"""Record complete syntax exceptions for the native parser's existing error corpus."""
import ast
import json
from pathlib import Path
import sys


def main():
    root=Path(__file__).absolute().parents[3]
    folder=root/'native/src/code_rule_dsl/facts/vtab/syntax/python_syntax'
    if sys.version_info[:2]!=(3,13):raise ValueError('Record with pinned Python 3.13')
    rows=json.loads((folder/'python-errors.json').read_text())
    extras=[
        'é = 1; x =\n', 'é = "😀"\nx =\n', 'def f():', 'def f():\n   ', 'def f(\n',
        'if True:\n\tpass\n        pass\n',
        'x = [foo.bar 2]\n', 'x = [f(1) 2]\n', 'x = ["a" "b" 3]\n',
        'x = [\n 1)\n', 'x = (\n 1\n', 'x = 1)\n',
        'x = "é\n', 'x = f"{value!z}"\n', 'x = f"{value"\n',
        'def f(a=1, b): pass\n', 'def f(**a,b): pass\n',
        'if True:\r\npass\r\n', 'value = 1 \\ x\n', 'value =\x00',
        'def f(:\n', 'def another(α:\n', 'if True:\n    pass\n  value\n',
    ]
    rows.extend(dict(source=source) for source in extras)
    for row in rows:
        try:ast.parse(row['source'],filename='input.py')
        except SyntaxError as error:
            row['exception']=dict(kind=type(error).__name__,message=error.msg,line=error.lineno,offset=error.offset,
                endLine=error.end_lineno,endOffset=error.end_offset,text=error.text)
        else:raise AssertionError('Error fixture parsed successfully')
    (folder/'python-exceptions.json').write_text(json.dumps(rows,indent=2,ensure_ascii=False)+'\n')


if __name__=='__main__':main()
