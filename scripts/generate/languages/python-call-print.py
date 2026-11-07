"""Record native call-printing expectations from pinned CPython, offline."""
import ast,json,sys
from pathlib import Path


def main():
    if sys.version_info[:2]!=(3,13):raise ValueError('Use pinned CPython 3.13')
    root=Path(__file__).resolve().parents[3]
    directory=root/'native/src/code_rule_dsl/facts/vtab/syntax/python_print'
    sources=[row['source'] for row in json.loads((directory/'expressions.json').read_text())]
    sources.extend([
      'factory ( VersionedScope, # lifecycle\n name="_plan_owner" )\n',
      'f(x for x in values)\n','(factory())(arg)\n','((obj.method))(x)\n',
      '(lambda x: x)(arg)\n','f(*xs, key=True, **kwargs)\n',
      'ｆ(ｏｂｊｅｃｔ, Ｖｅｒｓｉｏｎｅｄ)\n',
      'getattr(obj, "_pl\\u0061n_owner")\n',
      'f({"Versioned": [g(x), h(y)]}, x=(z := 1))\n',
      'f("_base" "_scope")\n','(registry["imported_contract"])(CtePublicationContract)\n',
    ])
    rows=[]
    for source in sources:
        calls=sorted((n for n in ast.walk(ast.parse(source)) if isinstance(n,ast.Call)),
            key=lambda n:(n.lineno,n.col_offset,n.end_lineno,n.end_col_offset))
        rows.append(dict(source=source,calls=[[ast.unparse(n),ast.unparse(n.func)] for n in calls]))
    (directory/'calls.json').write_text(json.dumps(rows,indent=2,ensure_ascii=True)+'\n')
    print(len(rows),'source cases;',sum(len(row['calls']) for row in rows),'canonical call pairs')


if __name__=='__main__':main()
