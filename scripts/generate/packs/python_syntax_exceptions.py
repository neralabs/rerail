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
    for row in rows:
        try:ast.parse(row['source'],filename='input.py')
        except SyntaxError as error:
            row['exception']=dict(kind=type(error).__name__,message=error.msg,line=error.lineno,offset=error.offset,
                endLine=error.end_lineno,endOffset=error.end_offset,text=error.text)
        else:raise AssertionError('Error fixture parsed successfully')
    (folder/'python-exceptions.json').write_text(json.dumps(rows,indent=2,ensure_ascii=False)+'\n')


if __name__=='__main__':main()
